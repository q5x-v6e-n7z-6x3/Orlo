; Node names match tree-sitter-hcl 1.1.0's bundled node-types.json.
(identifier) @variable
(block (identifier) @type)
(attribute (identifier) @property)
(function_call (identifier) @function)
(get_attr (identifier) @property)
(comment) @comment
[(string_lit) (quoted_template) (heredoc_template)] @string
(numeric_lit) @number
[(bool_lit) (null_lit)] @constant.builtin
["for" "in" "if" "else" "endif" "endfor"] @keyword
["{" "}" "[" "]" "(" ")"] @punctuation.bracket
["." "," ":"] @punctuation.delimiter
["=" "+" "-" "*" "/" "%" "!" "&&" "||" "==" "!=" "<" ">" "<=" ">=" "?" "=>"] @operator
