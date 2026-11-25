import json
from pathlib import Path
import subprocess
import sys
import unittest
from urllib.request import Request, urlopen

from app import procs, types


class DemoTests(unittest.TestCase):
    def test_procs(self):
        self.assertEqual(procs.run_calc(3), 115)
        self.assertEqual(procs.run_classify(types.Expr.Neg(types.Expr.Lit(2))), "negated literal")
        self.assertEqual(procs.run_main(2, "hi"), "end:S")

    def test_http(self):
        server = Path(__file__).with_name("api.py")
        with subprocess.Popen(
            [sys.executable, str(server), "--addr", "127.0.0.1:0"],
            stderr=subprocess.PIPE,
            text=True,
        ) as process:
            try:
                ready = process.stderr.readline().strip()
                self.assertTrue(ready.startswith("listening on http://"), ready)
                base = ready.removeprefix("listening on ")
                with urlopen(base + "/", timeout=5) as response:
                    self.assertIn(b"<!doctype html>", response.read().lower())
                with urlopen(base + "/api/hello?name=vandorf&times=2", timeout=5) as response:
                    self.assertEqual(json.load(response), "hello, vandorf! hello, vandorf!")
                request = Request(
                    base + "/api/measure", data=b'{"shapes":["Dot",{"Circle":1},{"Rect":{"w":2,"h":3}}]}',
                    headers={"Content-Type": "application/json"},
                )
                with urlopen(request, timeout=5) as response:
                    report = json.load(response)
                    self.assertEqual(report["count"], 3)
                    self.assertEqual(report["kinds"], {"dot": 1, "circle": 1, "rect": 1})
                    self.assertAlmostEqual(report["total"], 9.141592653589793)
            finally:
                process.terminate()
