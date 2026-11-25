from math import pi
from threading import Lock

from app import types


def range_values(i: int) -> types.ForEachStartOut:
    return types.ForEachStartOut(l=tuple(range(i)), s=str(i))


def square(i: int, _l: tuple[int, ...]) -> int:
    return i * i


def sum_values(l: tuple[int, ...]) -> int:
    return sum(l)


def repeat(a: int, b: str) -> str:
    return b * a


_grown = 0
_lock = Lock()
_logged = []


def grow(s: str) -> int:
    global _grown
    with _lock:
        _grown += 3
        return len(s.encode("utf-8")) + _grown


def describe(i: int) -> str:
    return f"sum={i}"


def log(i: int) -> None:
    with _lock:
        _logged.append(i)


def finish(r: types.Outcome) -> str:
    match r:
        case types.Outcome.I(_):
            return "end:I"
        case types.Outcome.S():
            return "end:S"
        case types.Outcome.B(_):
            return "end:B"
    raise ValueError("unknown outcome")


def call(s: str) -> int:
    return len(s.encode("utf-8"))


def args(a: int, b: str) -> types.BarArgs:
    return types.BarArgs(a=a, b=b)


def build(base: int) -> types.Expr:
    return types.Expr.Add(types.ExprAdd(l=types.Expr.Lit(base), r=types.Expr.Lit(2)))


def _eval(e: types.Expr) -> int:
    match e:
        case types.Expr.Lit(i):
            return i
        case types.Expr.Neg(x):
            return -_eval(x)
        case types.Expr.Add(a):
            return _eval(a.l) + _eval(a.r)
    raise ValueError("unknown expression")


def eval(e: types.Expr, scale: int) -> int:
    return _eval(e) * scale


def total(xs: tuple[int, ...]) -> int:
    return sum(xs)


def look(e: types.Expr) -> types.Expr:
    return e


def small(i: int) -> bool:
    return abs(i) < 100


def declare(names: tuple[str, ...], query: str) -> types.ResolveDeclareOut:
    scope = {n: types.ScopeValue(slot=i, kind="var") for i, n in enumerate(names)}
    return types.ResolveDeclareOut(scope=scope, query=query)


def adjust(scope: types.Scope, query: str) -> types.ResolveDeclareOut:
    return types.ResolveDeclareOut(scope=scope, query=query)


def found(name: str, slot: int, kind: str, total: int) -> str:
    return f"{name}: slot {slot} ({kind}) of {total}"


def missing(name: str, known: tuple[str, ...]) -> str:
    return f"{name}: not declared; known: {', '.join(known)}"


def shapes(shapes: tuple[types.Shape, ...]) -> tuple[types.Shape, ...]:
    return shapes


def area(s: types.Shape) -> types.Measured:
    match s:
        case types.Shape.Dot():
            return types.Measured(kind="dot", area=0.0)
        case types.Shape.Circle(r):
            return types.Measured(kind="circle", area=pi * r * r)
        case types.Shape.Rect(r):
            return types.Measured(kind="rect", area=r.w * r.h)
    raise ValueError("unknown shape")


def step(f: types.Fold) -> types.Fold:
    return f


def greet(name: str, times: int) -> str:
    return (f"hello, {name}! " * times).rstrip()
