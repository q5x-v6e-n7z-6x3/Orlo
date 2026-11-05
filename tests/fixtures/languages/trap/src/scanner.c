#include "tree_sitter/parser.h"

// Traps on a `!` that starts a token, so tests can see a grammar fail mid-parse.
void *tree_sitter_trap_external_scanner_create(void) { return NULL; }
void tree_sitter_trap_external_scanner_destroy(void *payload) {}
unsigned tree_sitter_trap_external_scanner_serialize(void *payload, char *buffer) { return 0; }
void tree_sitter_trap_external_scanner_deserialize(void *payload, const char *buffer, unsigned length) {}
bool tree_sitter_trap_external_scanner_scan(void *payload, TSLexer *lexer, const bool *valid_symbols) {
  while (lexer->lookahead == ' ' || lexer->lookahead == '\n') lexer->advance(lexer, true);
  if (lexer->lookahead == '!') __builtin_trap();
  return false;
}
