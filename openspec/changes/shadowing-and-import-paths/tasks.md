## 1. Shadowing (D1, D2)

- [ ] 1.1 Turn the `local-duplicating-a-parameter` fixture into `// expect: ok` without the `FIXME(decision)`, and add a fixture showing the local is the one used (a `String` local shadowing an `Int` parameter, returned from a `String` function).
- [ ] 1.2 Remove the `is_imported` check from `define_top_level`. Update the regression tests and fixtures that expected `E4010` for an import collision. Add multi-file fixtures for every scenario of "Ambiguous Imported Names Rejected at Use", including a local declaration shadowing a name that two imports export.

## 2. Import paths (D3)

- [ ] 2.1 Report `.`, `..` and relative segments in `parse_import_decl` with the D3 message and span. Turn the three `import-attempting-directory-traversal--*` fixtures into `// expect: E2001` without `FIXME(decision)`, and assert the message in a parser test.
- [ ] 2.2 Update the `E1006` row in `openspec/specs/diagnostics/spec.md` to "a file importing itself, or a sibling import with more than one segment; `.`/`..` segments are rejected by the parser (`E2001`)".

## 3. Integration

- [ ] 3.1 Run the apply guidance in `openspec/config.yaml` (fmt, clippy, tests, examples, `openspec validate --all --strict`), and tick the three matching decision items in `ROADMAP.md`.
