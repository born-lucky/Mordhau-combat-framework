# speckit.adapters - the populate framework: a populate run is a list of adapters, each filling one part of the Book
# from one kind of source. The kit fixes the order and the contract; a game supplies the adapters.
#
#   kind        what it adds                                              typical source
#   source      Sources rows (usually implicit: Book.source inside the others)
#   entity      Entities + T_<type> values (Book.entity / Book.set)       a verified reader's record dump (JSON)
#   field       Fields metadata after the values exist (source line,      the record classes' declarations; unit rules
#               description, unit + unit_source)
#   rule        Rules rows (generated from the port's citations, plus     `Name rva=0x..` comments in the port; bytecode
#               curated composite rules with params read back from code)  dumps; tables read back from the port
#   evidence    Evidence rows (per field / rule ledger + measurements)     test files; parity results
#   finish      anything that needs the complete book (tiers)             speckit.tiers
#
# Order: entity -> field -> rule -> evidence -> finish (each kind sees everything the earlier kinds made).
# Rules of the method every adapter keeps: never type a value in (read it), cite the source on the row, use
# permanent IDs (Book.entity / sid), and fail loudly (SystemExit) on a malformed input instead of skipping it.
ORDER = ("entity", "field", "rule", "evidence", "finish")


class Adapter:
    """kind: one of ORDER; name: for the log; run(book, ctx) fills the book (ctx: a dict shared by all adapters)"""
    kind = "entity"
    name = ""

    def run(self, book, ctx):
        raise NotImplementedError


class FnAdapter(Adapter):
    """an adapter from a plain function f(book, ctx)"""

    def __init__(self, kind, fn, name=""):
        assert kind in ORDER, kind
        self.kind, self.fn, self.name = kind, fn, name or getattr(fn, "__name__", kind)

    def run(self, book, ctx):
        return self.fn(book, ctx)


class Pipeline:
    def __init__(self, adapters):
        self.adapters = list(adapters)
        for a in self.adapters:
            if a.kind not in ORDER:
                raise SystemExit(f"adapter {a.name}: unknown kind {a.kind}")

    def populate(self, book, ctx=None):
        """run every adapter, kinds in ORDER, adapters of one kind in the order given"""
        ctx = {} if ctx is None else ctx
        for kind in ORDER:
            for a in self.adapters:
                if a.kind == kind:
                    a.run(book, ctx)
        return book
