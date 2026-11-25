"""Report callable signatures without importing the implementation module."""
import ast
import sys
import tokenize

with tokenize.open(sys.argv[1]) as source:
    tree = ast.parse(source.read(), filename=sys.argv[1])

for item in tree.body:
    if isinstance(item, (ast.FunctionDef, ast.AsyncFunctionDef)):
        args = item.args
        argc = len(args.posonlyargs) + len(args.args)
        minimum = argc - len(args.defaults)
        maximum = "*" if args.vararg else argc
        async_or_keyword_only = isinstance(item, ast.AsyncFunctionDef) or None in args.kw_defaults
        print(item.name, minimum, maximum, int(async_or_keyword_only), sep="\t")
