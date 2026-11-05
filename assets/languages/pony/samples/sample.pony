"""
A counter that reports to the console.
"""
use "collections"
use @printf[I32](fmt: Pointer[U8] tag, ...)

/* block comment /* nested */ still a comment */
type Number is (I64 | F64)

actor Main
  let _env: Env
  var _count: U64 = 0

  new create(env: Env) =>
    """ Start counting. """
    _env = env
    let counter = Counter(3)
    counter.increment(where by = 2)
    _report(counter.total())
    @printf("%s\n".cstring(), __loc.method_name().cstring())

  be tick(times: U32) =>
    var i: U32 = 0
    while i < times do
      i = i + 1
    end
    for n in Range(0, 10) do
      if (n % 2) == 0 then
        _count = _count + 1
      elseif n > 7 then
        break
      else
        continue
      end
    end

  fun ref _report(total: Number) =>
    match total
    | let n: I64 if n > 0 => _env.out.print("positive " + n.string())
    | let f: F64 => _env.out.print("float " + f.string())
    else
      _env.out.print("none")
    end

class Counter
  var _total: I64
  let _step: I64

  new create(step: I64) =>
    _total = 0
    _step = step

  fun ref increment(by: I64 = 1) =>
    _total = _total + (by * _step)

  fun total(): I64 => _total

primitive Maths
  fun half(x: F64): F64 => x / 2.0e0

  fun parse(text: String): I64 ? =>
    try text.i64()? else error end

interface Named
  fun name(): String val

trait Greeter is Named
  fun greet(): String => "Hello, " + name() + '!'.string() + "\t\"quoted\""

struct Point
  var x: F32 = 1.5
  var y: F32 = 0x10.f32()

  fun ref apply(f: {(F32): F32} val) => x = f(x)

  fun clone(): Point iso^ =>
    recover Point end

  fun is_origin(): Bool => (x == 0) and (y == 0) or false

  fun repeated(): U8 =>
    var n: U8 = 0
    repeat n = n + 1 until n > 3 end
    n
