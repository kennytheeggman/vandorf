//! vcc's manifest check refuses what the reference's serde types refuse,
//! with the same message.

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
    "[project]\nname = \"x\"\n[rust]\nout = \"a\"\npost = \"cargo build\"\n",
    "[project]\nname = \"x\"\n[rust]\nout = \"a\"\ntest = [1]\n",
    "[project]\nname = \"x\"\n[go]\nout = \"a\"\ntest = \"go test\"\n",
];

#[test]
fn manifest_checks_match_the_reference() {
    let mut bad = vec![];
    for text in CASES {
        let reference = toml::from_str::<vandorf::project::Manifest>(text).err().map(|e| e.message().to_string()).unwrap_or_default();
        let mine = vcc::vcc::run_check_manifest((*text).into()).unwrap().to_string();
        if reference != mine {
            bad.push(format!("{text:?}\n  reference: {reference:?}\n  vcc:       {mine:?}"));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
