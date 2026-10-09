//! The main menu's news panel: `UMordhauNewsWidget` (Mordhau\Public\Components\MordhauNewsWidget.h) ported from the
//! exe, plus the engine's `UAsyncTaskDownloadImage` the BP uses for the pictures.
//!
//! - `UMordhauNewsWidget::UMordhauNewsWidget` rva 0x14af700: NewsURL "https://mordhau.com/news/en/plain/", Verb "GET",
//!   FileName "News.json" (FileName is never read: there is no on-disk cache, so offline the panel stays empty).
//! - `UMordhauNewsWidget::SendHTTPRequest` rva 0x14d2c60: FHttpModule CreateRequest, SetURL(NewsURL), SetVerb(Verb),
//!   OnProcessRequestComplete -> OnRequestComplete, ProcessRequest; returns true.
//! - `UMordhauNewsWidget::OnRequestComplete` rva 0x14ce970: unsuccessful -> HandleAndBroadcastErrorMessage("News
//!   request unsuccessful ..."); content shorter than one character -> "Empty content at this URL"; else
//!   ClearMordhauMLData, `DumbMordhauMDParser::Parse` rva 0x1511060 into MordhauMLData, and FOnRefreshCompleted
//!   broadcast (MordhauMLData, ResponseCode, ElapsedTime).
//! - `UMordhauNewsWidget::HandleAndBroadcastErrorMessage` rva 0x14bfda0: logs, ClearMordhauMLData, broadcasts the
//!   (empty) data with the code and time.
//! - The BP (state/ui_kismet/BP_MordhauNewsWidget.txt) then queues the image URLs (Prepare Download Queue), downloads
//!   them one by one (DownloadImage -> OnSuccess / OnFail) and fills its MordhauML_ItemWidget children (UpdateUI).
//!
//! The request is answered from a local document (user decision: no network) and `poll` delivers it to the VM on
//! the next frame, as the engine's HTTP manager ticks completions on the game thread.

use crate::model::{Id, V};
use crate::vm::Vm;
use std::sync::mpsc::{channel, Receiver};
use std::sync::Mutex;

/// `UMordhauNewsWidget::UMordhauNewsWidget` rva 0x14af700 defaults
pub const NEWS_URL: &str = "https://mordhau.com/news/en/plain/";
pub const NEWS_VERB: &str = "GET";

/// (response code, body, elapsed seconds) or an error text
type HttpResult = Result<(i64, Vec<u8>, f64), String>;

enum Kind {
    /// a UMordhauNewsWidget's SendHTTPRequest
    News,
    /// a UAsyncTaskDownloadImage (the task object)
    Image,
}

struct Job {
    vm: usize,
    obj: Id,
    kind: Kind,
    rx: Receiver<HttpResult>,
}

static JOBS: Mutex<Vec<Job>> = Mutex::new(Vec::new());

/// a stable key for a VM (its address changes when the host moves the UiRuntime into a resource)
fn vm_key(vm: &mut Vm) -> usize {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);
    *vm.world.entry("__news_vm_key").or_insert_with(|| NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) as Id) as usize
}

/// The rewrite's news document (user decision 2026-10-06: no network; the panel shows our own "Mordhau Rewrite" news
/// in the markdown the exe parses). `$MH_NEWS_FILE` overrides the built-in news/news.md.
pub const NEWS_DOC: &str = include_str!("../news/news.md");

/// the request: NewsURL is answered from the local document (200), any other URL (the BP's image downloads) fails
/// as an offline request does (no connection: OnFail)
fn get(url: String) -> Receiver<HttpResult> {
    let (tx, rx) = channel();
    let r = if url == NEWS_URL {
        let doc = std::env::var_os("MH_NEWS_FILE").and_then(|p| std::fs::read(p).ok()).unwrap_or_else(|| NEWS_DOC.as_bytes().to_vec());
        Ok((200, doc, 0.0))
    } else {
        Err(format!("no network: {url}"))
    };
    let _ = tx.send(r);
    rx
}

/// `UMordhauNewsWidget::SendHTTPRequest` rva 0x14d2c60
pub fn send_http_request(vm: &mut Vm, w: Id) -> bool {
    let url = match vm.prop(w, "NewsURL") {
        V::None => NEWS_URL.to_string(),
        v => v.s(),
    };
    let rx = get(url);
    if let Ok(mut j) = JOBS.lock() {
        j.push(Job { vm: vm_key(vm), obj: w, kind: Kind::News, rx });
    }
    true
}

/// `UAsyncTaskDownloadImage::DownloadImage` (UE 4.26 AsyncTaskDownloadImage.cpp): a new task object that starts the
/// GET at once; its OnSuccess / OnFail fire when the request completes
pub fn download_image(vm: &mut Vm, url: &str) -> V {
    let c = vm.native_class("AsyncTaskDownloadImage");
    let task = vm.new_obj(c, "AsyncTaskDownloadImage");
    let rx = get(url.to_string());
    if let Ok(mut j) = JOBS.lock() {
        j.push(Job { vm: vm_key(vm), obj: task, kind: Kind::Image, rx });
    }
    V::Obj(task)
}

fn broadcast(vm: &mut Vm, obj: Id, prop: &str, args: Vec<V>) {
    if let V::Multi(m) = vm.prop(obj, prop) {
        for (o, f) in m {
            vm.call_named(o, &f, args.clone());
        }
    }
}

/// deliver finished requests (called once per frame by the host before the VM ticks)
pub fn poll(vm: &mut Vm) {
    let me = vm_key(vm);
    let done: Vec<(Id, Kind, HttpResult)> = {
        let Ok(mut jobs) = JOBS.lock() else { return };
        let mut out = vec![];
        let mut keep = vec![];
        for j in jobs.drain(..) {
            if j.vm != me {
                keep.push(j);
                continue;
            }
            match j.rx.try_recv() {
                Ok(r) => out.push((j.obj, j.kind, r)),
                Err(std::sync::mpsc::TryRecvError::Empty) => keep.push(j),
                Err(_) => out.push((j.obj, j.kind, Err("request dropped".into()))),
            }
        }
        *jobs = keep;
        out
    };
    for (obj, kind, r) in done {
        if !vm.alive(obj) {
            continue;
        }
        match kind {
            Kind::News => on_request_complete(vm, obj, r),
            Kind::Image => on_image(vm, obj, r),
        }
    }
}

/// `UMordhauNewsWidget::OnRequestComplete` rva 0x14ce970 (+ HandleAndBroadcastErrorMessage rva 0x14bfda0)
fn on_request_complete(vm: &mut Vm, w: Id, r: HttpResult) {
    let (ml, code, elapsed) = match r {
        Ok((code, body, el)) => {
            let text = String::from_utf8_lossy(&body).into_owned();
            if text.is_empty() {
                bevy::log::warn!("LogNewsWidget: Empty content at this URL");
                (empty_ml(), code, el)
            } else {
                (parse(&text), code, el)
            }
        }
        Err(e) => {
            bevy::log::warn!("LogNewsWidget: News request unsuccessful: {e}");
            (empty_ml(), 0, 0.0)
        }
    };
    vm.set(w, "MordhauMLData", ml.clone());
    broadcast(vm, w, "FOnRefreshCompleted", vec![ml, V::Int(code), V::Float(elapsed)]);
}

/// `UAsyncTaskDownloadImage::HandleImageRequest` (UE 4.26): a 200 image -> OnSuccess(UTexture2DDynamic), else
/// OnFail(nullptr)
fn on_image(vm: &mut Vm, task: Id, _r: HttpResult) {
    // the rewrite downloads nothing (see `get`): every image request fails as offline -> OnFail(nullptr)
    // (UNCONFIRMED: no local image support; the news document has no pictures)
    broadcast(vm, task, "OnFail", vec![V::None]);
}

// ---- DumbMordhauMDParser (rva 0x1511060 and its Check* helpers) ------------------------------------------------

fn s(x: &str) -> V {
    V::Str(x.to_string())
}

fn empty_ml() -> V {
    V::st(&[("Header", V::st(&[("Version", s("")), ("Debug", s("")), ("Platforms", V::Array(vec![]))])), ("FontStyles", V::Array(vec![])), ("Widgets", V::Array(vec![]))])
}

#[derive(Clone, Default, Debug, PartialEq)]
pub struct MlText {
    pub data: String,
    pub multiline: Vec<String>,
    pub style: String,
    pub multiline_style: String,
}

#[derive(Clone, Default, Debug, PartialEq)]
pub struct MlWidget {
    pub src: String,
    pub text: MlText,
}

#[derive(Clone, Default, Debug, PartialEq)]
pub struct Ml {
    /// FMordhauMLFontStlye names in order, with their Alignment ("Left", "Center" for Subtitle)
    pub font_styles: Vec<(String, String)>,
    pub widgets: Vec<MlWidget>,
}

struct Re {
    header: fancy_regex::Regex,
    semibold: fancy_regex::Regex,
    bold: fancy_regex::Regex,
    italic: fancy_regex::Regex,
    list: fancy_regex::Regex,
    image: fancy_regex::Regex,
    url: fancy_regex::Regex,
    alt: fancy_regex::Regex,
}

fn re() -> &'static Re {
    static R: std::sync::OnceLock<Re> = std::sync::OnceLock::new();
    R.get_or_init(|| {
        let r = |p: &str| fancy_regex::Regex::new(p).expect("news regex");
        Re {
            // CheckHeader rva 0x14f2fe0: .rdata 0x144337908
            header: r(r"(#+ )"),
            // Parse rva 0x1511060: the three emphasis passes, strongest first
            semibold: r(r"\*\*\*(?![*\s])(?:[^*]*[^*\s])?\*\*\*"),
            bold: r(r"\*\*(?![*\s])(?:[^*]*[^*\s])?\*\*"),
            italic: r(r"\*(?![*\s])(?:[^*]*[^*\s])?\*"),
            list: r(r"^( *(\* |- ))"),
            // CheckImage rva 0x14f3750: .rdata 0x144337920, then the URL and alt-text patterns
            image: r(r#"!\[[^\]]*\]\((.*?)\s*("(?:.*[^"])")?\s*\)"#),
            url: r(r"\(([a-zA-Z0-9\:\/\.\?\-\_]+\))"),
            alt: r(r"(?P<alt>!\[[^\]]*\])"),
        }
    })
}

fn matches(re: &fancy_regex::Regex, s: &str) -> Vec<(usize, usize)> {
    re.find_iter(s).filter_map(|m| m.ok()).map(|m| (m.start(), m.end())).collect()
}

/// CheckHeader rva 0x14f2fe0: "(#+ )" anywhere; at the line start the style is "H<n>" (n = number of '#'), the text
/// what follows. A match elsewhere formats an "Normal<n>" style (the exe passes a narrow "Normal" to a wide %s:
/// UNCONFIRMED string, the BP falls back to StyleNormal for it either way)
fn check_header(line: &str, pure: &mut String, style: &mut String) -> bool {
    let ms = matches(&re().header, line);
    for &(b, e) in &ms {
        let n = e - b - 1;
        *style = if b == 0 { format!("H{n}") } else { format!("Normal{n}") };
        *pure = line[e..].to_string();
    }
    !ms.is_empty()
}

/// CheckTypographicalEmphasis rva 0x14f44a0: each match of the pattern restarts from the input line, cuts the match out
/// and inserts Tag + match-without-'*' + "</>" in its place (so with several matches only the last one is converted,
/// as in the exe)
fn check_emphasis(line: &str, pure: &mut String, re: &fancy_regex::Regex, tag: &str) -> bool {
    let ms = matches(re, line);
    for &(b, e) in &ms {
        let affected = &line[b..e];
        let mut out = line.to_string();
        out.replace_range(b..e, "");
        let trimmed = format!("{tag}{}</>", affected.replace('*', ""));
        out.insert_str(b, &trimmed);
        *pure = out;
    }
    !ms.is_empty()
}

/// CheckUnOrderedListItem rva 0x14f4790: the bullet ("* " / "- " after optional spaces) cut from the line
fn check_list(line: &str, pure: &mut String) -> bool {
    let ms = matches(&re().list, line);
    for &(b, e) in &ms {
        let mut out = line.to_string();
        out.replace_range(b..e, "");
        *pure = out;
    }
    !ms.is_empty()
}

/// CheckImage rva 0x14f3750: `![alt](url "title")`: the text before and after, the URL (its "(...)" group with the
/// parentheses removed) as Image.Src, the alt text (without "![" / "]") as Text.Data in the "Subtitle" style
fn check_image(line: &str, before: &mut String, after: &mut String, w: &mut MlWidget) -> bool {
    let ms = matches(&re().image, line);
    for &(b, e) in &ms {
        let affected = &line[b..e];
        *before = line[..b].to_string();
        *after = line[e..].to_string();
        for (ub, ue) in matches(&re().url, affected) {
            w.src = affected[ub..ue].replace('(', "").replace(')', "");
        }
        for (ab, ae) in matches(&re().alt, affected) {
            w.text.data = affected[ab..ae].replace("![", "").replace(']', "");
            w.text.style = "Subtitle".into();
        }
    }
    !ms.is_empty()
}

/// AddFontStyle rva 0x14eec20: a style name not seen yet becomes an FMordhauMLFontStlye (ctor rva 0x14e5230: Size 18,
/// TypeFace "CrimsonText Regular", Alignment "Left", padding 0) with Alignment "Center" for "Subtitle"
fn add_font_style(ml: &mut Ml, name: &str) {
    if ml.font_styles.iter().any(|(n, _)| n == name) {
        return;
    }
    let align = if name == "Subtitle" { "Center" } else { "Left" };
    ml.font_styles.push((name.to_string(), align.to_string()));
}

/// `DumbMordhauMDParser::Parse` rva 0x1511060
pub fn parse_ml(md: &str) -> Ml {
    let mut ml = Ml::default();
    // FString::ParseIntoArrayLines(InStringMD, bCullEmpty = true): "\r\n", "\r", "\n"
    let lines: Vec<&str> = md.split("\r\n").flat_map(|l| l.split(['\r', '\n'])).filter(|l| !l.is_empty()).collect();
    for line in lines {
        let mut lt = MlText { data: line.to_string(), ..Default::default() };
        let is_header = check_header(line, &mut lt.data, &mut lt.style);
        for (re, tag) in [(&re().semibold, "<SemiBold>"), (&re().bold, "<Bold>"), (&re().italic, "<Italic>")] {
            let cur = lt.data.clone();
            check_emphasis(&cur, &mut lt.data, re, tag);
        }
        let cur = lt.data.clone();
        let is_list = check_list(&cur, &mut lt.data);
        let mut push_line = true;
        if is_list {
            match ml.widgets.last_mut() {
                Some(last) if !last.text.multiline.is_empty() => {
                    last.text.multiline.push(lt.data.clone());
                    add_font_style(&mut ml, &lt.style);
                    continue;
                }
                _ => {
                    // the first item of a list starts a widget of its own
                    lt.multiline.push(lt.data.clone());
                    lt.data.clear();
                    lt.multiline_style = "Normal".into();
                }
            }
        } else {
            if !is_header {
                lt.style = "Normal".into();
            }
            let (mut before, mut after) = (String::new(), String::new());
            let mut img = MlWidget::default();
            if check_image(&lt.data.clone(), &mut before, &mut after, &mut img) {
                if !before.is_empty() {
                    ml.widgets.push(MlWidget { src: String::new(), text: MlText { data: before, style: "Normal".into(), ..Default::default() } });
                }
                let st = img.text.style.clone();
                ml.widgets.push(img);
                add_font_style(&mut ml, &st);
                if !after.is_empty() {
                    ml.widgets.push(MlWidget { src: String::new(), text: MlText { data: after, style: "Normal".into(), ..Default::default() } });
                }
                push_line = false;
            }
        }
        add_font_style(&mut ml, &lt.style);
        if push_line {
            ml.widgets.push(MlWidget { src: String::new(), text: lt });
        }
    }
    ml
}

/// FMordhauML as the BP sees it (struct members by name)
pub fn ml_value(ml: &Ml) -> V {
    let styles = ml
        .font_styles
        .iter()
        .map(|(n, a)| {
            V::st(&[
                ("Name", s(n)),
                ("Size", V::Int(18)),
                ("TypeFace", s("CrimsonText Regular")),
                ("FontFamily", s("")),
                ("ColorRGBA", V::Array(vec![])),
                ("Alignment", s(a)),
                ("Padding", V::Float(0.0)),
                ("PaddingBottom", V::Float(0.0)),
            ])
        })
        .collect();
    let widgets = ml
        .widgets
        .iter()
        .map(|w| {
            V::st(&[
                (
                    "Image",
                    V::st(&[("Src", s(&w.src)), ("Name", s("")), ("Width", V::Int(0)), ("Height", V::Int(0)), ("X", V::Int(0)), ("Y", V::Int(0)), ("Alignment", s("")), ("Padding", V::Float(0.0))]),
                ),
                (
                    "Text",
                    V::st(&[
                        ("Data", s(&w.text.data)),
                        ("MultilineData", V::Array(w.text.multiline.iter().map(|x| s(x)).collect())),
                        ("DataFontStyle", s(&w.text.style)),
                        ("MultilineDataFontStyle", s(&w.text.multiline_style)),
                    ]),
                ),
            ])
        })
        .collect();
    V::st(&[("Header", V::st(&[("Version", s("")), ("Debug", s("")), ("Platforms", V::Array(vec![]))])), ("FontStyles", V::Array(styles)), ("Widgets", V::Array(widgets))])
}

pub fn parse(md: &str) -> V {
    ml_value(&parse_ml(md))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_news_markdown() {
        let md = "# Welcome to **MORDHAU**\n\nPrepare yourselves.\n\n## **INFO AND TIPS**\n* Finishing the tutorial\n* Enable team-markers\n![Banner](https://mordhau.com/img/a.png)\ntext ***x*** end\n";
        let ml = parse_ml(md);
        let w = &ml.widgets;
        assert_eq!(w[0].text.data, "Welcome to <Bold>MORDHAU</>");
        assert_eq!(w[0].text.style, "H1");
        assert_eq!(w[1].text.data, "Prepare yourselves.");
        assert_eq!(w[1].text.style, "Normal");
        assert_eq!(w[2].text.data, "<Bold>INFO AND TIPS</>");
        assert_eq!(w[2].text.style, "H2");
        assert_eq!(w[3].text.multiline, vec!["Finishing the tutorial", "Enable team-markers"]);
        assert_eq!(w[3].text.multiline_style, "Normal");
        assert_eq!(w[4].src, "https://mordhau.com/img/a.png");
        assert_eq!(w[4].text.data, "Banner");
        assert_eq!(w[4].text.style, "Subtitle");
        assert_eq!(w[5].text.data, "text <SemiBold>x</> end");
        assert!(ml.font_styles.iter().any(|(n, a)| n == "Subtitle" && a == "Center"));
    }
}
