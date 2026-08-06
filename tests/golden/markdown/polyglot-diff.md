<!-- notignored-report -->

### notignored: 3 suppressions

- **ruff ANN001** — _the gateway fixes this signature_ — [api/service.py:14](https://github.com/acme/widgets/blob/0123456789abcdef0123456789abcdef01234567/api/service.py#L14)

  ```python
  12 |
  13 |
  14 | def fetch_all(ids):  # noqa: ANN001  # the gateway fixes this signature
  15 |     return [fetch(one) for one in ids]
  ```

- **rust dead\_code** — _error recovery lands with the next parser_ — [crates/lexer.rs:1](https://github.com/acme/widgets/blob/0123456789abcdef0123456789abcdef01234567/crates/lexer.rs#L1)

  ```rust
  1 | #[expect(dead_code, reason = "error recovery lands with the next parser")]
  2 | fn recover() {}
  ```

- **typescript (all rules)** — _the SDK's teardown hook is untyped_ — [web/widget.ts:17](https://github.com/acme/widgets/blob/0123456789abcdef0123456789abcdef01234567/web/widget.ts#L17)

  ```typescript
  15 | alert(sdk.consent());
  16 |
  17 | // @ts-expect-error the SDK's teardown hook is untyped
  18 | sdk.teardown();
  ```

