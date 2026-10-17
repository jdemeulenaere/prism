## 1. Shadowing (D1, D2)

- [x] 1.1 Turn the `local-duplicating-a-parameter` fixture into `// expect: ok` without the `FIXME(decision)`, and add a fixture showing the local is the one used (a `String` local shadowing an `Int` parameter, returned from a `String` function).
- [x] 1.2 Remove the `is_imported` check from `define_top_level`. Update the regression tests and fixtures that expected `E4010` for an import collision. Add multi-file fixtures for every scenario of "Ambiguous Imported Names Rejected at Use", including a local declaration shadowing a name that two imports export.

## 2. Import paths (D3)

- [x] 2.1 Report `.`, `..` and relative segments in `parse_import_decl` with the D3 message and span. Turn the three `import-attempting-directory-traversal--*` fixtures into `// expect: E2001` without `FIXME(decision)`, and assert the message in a parser test.
- [x] 2.2 Update the `E1006` row in `openspec/specs/diagnostics/spec.md` to "a file importing itself, or a sibling import with more than one segment; `.`/`..` segments are rejected by the parser (`E2001`)".

## 3. Annotations naming an unimported type (D4)

- [x] 3.1 Report `E4014` in `check_annotation_names` for a name known only through an import's signature, and bind it to the error type. Verify with regression tests for each annotation site and a multi-file fixture for "Naming an unimported type in an annotation".

## 4. Integration

- [ ] 4.1 Run the apply guidance in `openspec/config.yaml` (fmt, clippy, tests, examples, `openspec validate --all --strict`), and tick the four matching decision items in `ROADMAP.md`.
