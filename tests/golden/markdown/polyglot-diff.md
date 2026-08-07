<!-- notignored-report -->

### notignored: 3 suppressions

- **ruff ANN001** — _the gateway fixes this signature_ — [api/clean.py:4](https://github.com/acme/widgets/blob/0123456789abcdef0123456789abcdef01234567/api/clean.py#L4)

  <details>
  <summary>suppressed code</summary>

  ```python
  4 | def widths(rows):  # noqa: ANN001  # the gateway fixes this signature
  ```

  </details>

- **rust dead\_code** — _error recovery lands with the next parser_ — [crates/lexer.rs:1](https://github.com/acme/widgets/blob/0123456789abcdef0123456789abcdef01234567/crates/lexer.rs#L1)

  <details>
  <summary>suppressed code</summary>

  ```rust
  1 | #[expect(dead_code, reason = "error recovery lands with the next parser")]
  2 | fn recover() {}
  ```

  </details>

- **typescript (all rules)** — _the SDK's teardown hook is untyped_ — [web/widget.ts:17](https://github.com/acme/widgets/blob/0123456789abcdef0123456789abcdef01234567/web/widget.ts#L17)

  <details>
  <summary>suppressed code</summary>

  ```typescript
  18 | sdk.teardown();
  ```

  </details>

