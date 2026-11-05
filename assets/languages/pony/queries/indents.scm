[
  (use_statement)
  (actor_definition)
  (class_definition)
  (primitive_definition)
  (interface_definition)
  (trait_definition)
  (struct_definition)

  (constructor)
  (method)
  (behavior)

  (parameters)

  (if_block)
  (then_block)
  (elseif_block)
  (else_block)
  (iftype_statement)
  (elseiftype_block)
  (do_block)
  (match_statement)
  (parenthesized_expression)
  (tuple_expression)

  (array_literal)
  (object_literal)
] @indent.begin

(try_statement (block) @indent.begin)

(repeat_statement (block) @indent.begin)

(recover_statement (block) @indent.begin)

(return_statement (block) @indent.begin)

(continue_statement (block) @indent.begin)

(break_statement (block) @indent.begin)

[
  "}"
  "]"
  ")"
] @indent.end

[ "{" "}" ] @indent.branch

[ "[" "]" ] @indent.branch

[ "(" ")" ] @indent.branch

; Orlo: blocks close with `end`, as in nvim-treesitter's Lua and Ruby queries
"end" @indent.end @indent.branch
