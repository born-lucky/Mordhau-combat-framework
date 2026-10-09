import tempfile
from pathlib import Path
import unittest
from audit_package import audit, errors_for

class AuditTests(unittest.TestCase):
    def check_blob(self, name, blob):
        with tempfile.TemporaryDirectory() as d:
            p=Path(d)/name
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_bytes(blob)
            return errors_for(p,name)

    def test_original_binary_hidden_as_source_rejected(self):
        self.assertTrue(self.check_blob("src/main.rs",b"MZoriginal game"))

    def test_generated_tables_rejected(self):
        self.assertTrue(self.check_blob("shadow_capsules.json",b"{}"))
        self.assertTrue(self.check_blob("data_gen/spec/index.json",b"{}"))

    def test_original_shaders_not_embeddable(self):
        self.assertTrue(self.check_blob("src/shader.rs",b'include_str!("uepost.wgsl")'))
        self.assertTrue(self.check_blob("uepost.wgsl",b"shader"))

    def test_generated_ignored_directory_is_still_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            p=Path(d)/"extract"
            p.mkdir()
            (p/"original.rs").write_text("content")
            errors,_=audit(Path(d))
            self.assertTrue(errors)

    def test_reviewed_sheet_reader_source_does_not_allow_spreadsheets(self):
        prefix="tools/local-import/matrix-source/tools/sheets/"
        self.assertFalse(self.check_blob(prefix+"reader.py",b"def read(): pass"))
        self.assertTrue(self.check_blob(prefix+"game.xlsx",b"original"))
        self.assertTrue(self.check_blob("other/sheets/reader.py",b"def read(): pass"))

    def test_normal_source_accepted(self):
        self.assertFalse(self.check_blob("src/main.rs",b"fn main() {}"))

    def test_own_news_document_accepted(self):
        self.assertFalse(self.check_blob("src/news.rs",b'include_str!("../news/news.md")'))

    def test_private_key_rejected(self):
        self.assertTrue(self.check_blob("src/main.rs",b"-----BEGIN " + b"OPENSSH " + b"PRIVATE KEY-----"))

if __name__=="__main__":
    unittest.main()
