//! vcc's manifest check: what it refuses and why, per case, against the
//! golden files (first recorded from the reference's serde types).

const CASES: &[&str] = &[
    "[project]\nname = \"x\"\n",
    "[project]\nname = \"x\"\nsources = [\"sys\", \"lib\"]\n[rust]\nout = \"rs/src/x\"\n[bin.a]\nentry = \"A\"\n[http.web]\naddr = \"127.0.0.1:1\"\nroutes = { \"POST /a\" = \"A\" }\nraw = { \"GET /\" = \"P\" }\nstatic = { \"/x\" = \"x.html\" }\nrust = \"bin/web.rs\"\n",
    "[project]\nname = \"x\"\n[go]\nout = \"go/x\"\npackage = \"x\"\nmodule = \"m\"\ntypes_dir = \"t\"\n",
    "[project]\nname = \"x\"\n[rust]\nout = \"a\"\npost = [\"cargo build\", \"echo done\"]\ntest = [\"cargo test\"]\n",
    "[project]\nname = \"x\"\n[go]\nout = \"a\"\npost = []\ntest = [\"go test ./...\"]\n",
    // Syntax.
    "[project\nname = \"x\"\n",
    "[project]\nname = \n",
    "[project]\nname = \"x\"\nname = \"y\"\n",
    // Missing and unknown fields.
    "",
    "[project]\n",
    "[project]\nname = \"x\"\n[rust]\n",
    "[project]\nname = \"x\"\n[http.web]\nroutes = {}\n",
    "[project]\nname = \"x\"\nsauces = []\n",
    "[project]\nname = \"x\"\n[go]\nout = \"a\"\nmodul = \"m\"\n",
    "[project]\nname = \"x\"\n[rust]\nout = \"a\"\nmod = \"m\"\n",
    "[project]\nname = \"x\"\n[bin.a]\nentri = \"A\"\n",
    "[project]\nname = \"x\"\n[http.w]\naddr = \"a\"\nroute = {}\n",
    "[project]\nname = \"x\"\n[project2]\n",
    "[project]\nname = \"x\"\n[http.w]\naddr = \"a\"\ngo = \"go/cmd/w\"\nrust = \"bin/w.rs\"\n",
    "[project]\nname = \"x\"\n[http.w]\naddr = \"a\"\ngo = 1\n",
    "[project]\nname = \"x\"\n[http.w]\naddr = \"a\"\ngoo = \"x\"\n",
    "[project]\nname = \"x\"\n[build]\npost = []\n",
    "[project]\nname = \"x\"\n[rust]\nout = \"a\"\npre = []\n",
    // Types.
    "[project]\nname = 1\n",
    "[project]\nname = true\n",
    "[project]\nname = 1.5\n",
    "[project]\nname = [\"x\"]\n",
    "[project]\nname = \"x\"\nsources = \"sys\"\n",
    "[project]\nname = \"x\"\nsources = [1]\n",
    "project = 1\n",
    "[project]\nname = \"x\"\nbin = 3\n",
    "[project]\nname = \"x\"\n[bin]\na = 1\n",
    "[project]\nname = \"x\"\n[http.w]\naddr = \"a\"\nroutes = { \"GET /\" = 1 }\n",
    "[project]\nname = \"x\"\n[http.w]\naddr = \"a\"\nraw = []\n",
    "[project]\nname = \"x\"\n[http.w]\naddr = \"a\"\nsetup = \"Setup\"\nroutes = { \"GET /\" = \"P\" }\n",
    "[project]\nname = \"x\"\n[http.w]\naddr = \"a\"\nsetup = 1\n",
    "[project]\nname = \"x\"\n[http.w]\naddr = \"a\"\nsetup = [\"Setup\"]\n",
    "[project]\nname = \"x\"\n[http.w]\naddr = \"a\"\nsetp = \"Setup\"\n",
    "[project]\nname = \"x\"\n[rust]\nout = \"a\"\npost = \"cargo build\"\n",
    "[project]\nname = \"x\"\n[rust]\nout = \"a\"\ntest = [1]\n",
    "[project]\nname = \"x\"\n[go]\nout = \"a\"\ntest = \"go test\"\n",
    // A server's cli.
    "[project]\nname = \"x\"\n[http.w]\naddr = \"a\"\ncli = \"Cli\"\n",
    "[project]\nname = \"x\"\n[http.w]\naddr = \"a\"\ncli = [\"Cli\"]\n",
];

mod common;

#[test]
fn checks_manifests() {
    for (i, text) in CASES.iter().enumerate() {
        let mine = vcc::vcc::run_check_manifest((*text).into()).unwrap().to_string();
        common::check("manifest", &format!("c{i:02}"), &format!("{text}\n---\n{mine}\n"));
    }
}
