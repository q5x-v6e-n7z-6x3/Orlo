## Shapes and their areas.
import std/[math, strutils]
from std/sequtils import mapIt

#[ block comment #[ nested ]# still a comment ]#
##[ documentation block ]##

type
  ShapeKind* = enum
    skCircle, skSquare
  Shape* = object
    case kind*: ShapeKind
    of skCircle:
      radius*: float
    of skSquare:
      side*: float
  Area = distinct float

const
  Pi2 = PI * 2
  Greeting = """Hello,
long string"""

let
  raw = r"C:\path"
  tab = '\t'
  hex = 0xFF'u8
  big = 1_000_000

var counter = 0

proc area*(shape: Shape): float =
  ## The shape's area.
  case shape.kind
  of skCircle:
    result = PI * shape.radius ^ 2
  of skSquare:
    result = shape.side * shape.side

func describe(shape: Shape): string =
  if shape.kind == skCircle and area(shape) > 1.0:
    "big circle"
  elif shape.kind == skCircle:
    "circle"
  else:
    "square: " & $shape.side

iterator countdown(n: int): int =
  var i = n
  while i > 0:
    yield i
    dec i

template twice(body: untyped) =
  body
  body

macro trace(x: untyped): untyped =
  result = x

proc main() =
  let shapes = @[Shape(kind: skCircle, radius: 1.5), Shape(kind: skSquare, side: 2.0)]
  for shape in shapes:
    echo describe(shape), " ", formatFloat(area(shape), ffDecimal, 2)
  try:
    discard parseInt("x")
  except ValueError as e:
    echo "bad: ", e.msg
  finally:
    inc counter
  twice:
    echo Greeting, raw, tab, hex, big
  echo shapes.mapIt(it.kind)
  block outer:
    for i in countdown(3):
      if i == 1: break outer

when isMainModule:
  main()
