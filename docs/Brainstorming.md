Scope:
- LLM look at a codebase
- Generate Unit tests
- Generate Property based tests
Requirements:
- Local LLMs
- No code hallucination

Software architectures:
- Unit Test generator
	- Inputs: Codebase, documentation, which function to test
	- Outputs: Test source files AND direct test operation
	- Structure
		- Step 0: Walk the dependency tree for LLM context with depth limits (Go) $\times$ the number of languages
			- Step 0.5: Generate JSON schema from dependency tree and verify (Go)
		- Step 1: Generate input output pairs for the function from LLM, and regenerate if non compliant with human verification (JS)
			- Step 1.5: Validate with property based testing (could be generated)
		- Step 2: Generate test source files from input output pairs (statically) (Java)
		- Step 3: Script to run the tests (statically) (Rust, or Ruby, or Makefile) $\times$ the number of languages
	- Supported languages:
		- C
		- Python <- this is gonna be hard
		- Java (Maven)
		- JavaScript (Node)
		- C#
		- Go
		- Rust
		- *Zig*
		- C++***
- Interface
	- CLI
	- Web to come
	- VSCode to come