"""Private backend configuration must participate in persistent cache identity."""
import tempfile
from pathlib import Path
import unittest
import build_vips

class RecipeTests(unittest.TestCase):
    def test_recipe_is_stable_but_tracks_options_and_codec_versions(self):
        with tempfile.TemporaryDirectory() as directory:
            prefix=Path(directory);pc=prefix/'lib/pkgconfig/vips.pc';pc.parent.mkdir(parents=True);pc.write_text('Name: vips\nVersion: 8.18.7\n')
            base={'version':'8.18.7','dependencies':{'libjpeg':'3.2'},'options':['release']}
            first=build_vips.write_build_record(prefix,base)
            self.assertEqual(first,build_vips.write_build_record(prefix,dict(reversed(list(base.items())))))
            self.assertNotEqual(first,build_vips.write_build_record(prefix,{**base,'options':['release','highway']}))
            self.assertNotEqual(first,build_vips.write_build_record(prefix,{**base,'dependencies':{'libjpeg':'3.3'}}))
            self.assertEqual(pc.read_text().count('photosort_recipe='),1)
            self.assertIn('Name: vips',pc.read_text())

    def test_cache_key_tracks_codec_versions(self):
        self.assertEqual(build_vips.cache_key({'libjpeg':'3.2','zlib':'1.3'}),build_vips.cache_key({'zlib':'1.3','libjpeg':'3.2'}))
        self.assertNotEqual(build_vips.cache_key({'libjpeg':'3.2'}),build_vips.cache_key({'libjpeg':'3.3'}))

if __name__=='__main__':unittest.main()
