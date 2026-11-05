; Node names match tree-sitter-kotlin-ng 1.1.0's bundled node-types.json.
(identifier) @variable
((identifier) @constant.builtin
  (#any-of? @constant.builtin "true" "false" "null"))
((identifier) @keyword
  (#any-of? @keyword "break" "continue"))
(user_type (identifier) @type)
(function_declaration name: (identifier) @function)
(parameter (identifier) @variable.parameter)
[(line_comment) (block_comment)] @comment
[(string_literal) (multiline_string_literal) (character_literal)] @string
(escape_sequence) @escape
[(number_literal) (float_literal)] @number
(annotation) @attribute
[
  "abstract" "actual" "annotation" "as" "as?" "by" "catch" "class" "companion"
  "const" "constructor" "crossinline" "data" "delegate" "do" "dynamic" "else"
  "enum" "expect" "external" "field" "file" "final" "finally" "for" "fun" "get"
  "if" "import" "in" "infix" "init" "inline" "inner" "interface" "internal" "is"
  "lateinit" "noinline" "object" "open" "operator" "out" "override" "package"
  "param" "private" "property" "protected" "public" "receiver" "return" "return@"
  "sealed" "set" "setparam" "super" "super@" "suspend" "tailrec" "this" "this@"
  "throw" "try" "typealias" "val" "value" "var" "vararg" "when" "where" "while"
] @keyword
["(" ")" "[" "]" "{" "}"] @punctuation.bracket
["." "," ":" ";" "::" "?."] @punctuation.delimiter
["=" "+" "-" "*" "/" "%" "!" "!!" "&&" "||" "==" "!=" "===" "!=="
 "<" ">" "<=" ">=" "+=" "-=" "*=" "/=" "%=" "++" "--" "->" "?:" ".." "..<"] @operator
