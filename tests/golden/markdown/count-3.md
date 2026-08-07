<!-- notignored-report -->

### notignored: 3 suppressions

- **eslint no-console** — _the banner is this program's output_ — [tests/fixtures/markdown/pair.ts:1](https://github.com/acme/widgets/blob/0123456789abcdef0123456789abcdef01234567/tests/fixtures/markdown/pair.ts#L1)

  <details>
  <summary>suppressed code</summary>

  ```typescript
  2 | console.log("notignored");
  ```

  </details>

- **typescript (all rules)** — _no reason given_ — [tests/fixtures/markdown/pair.ts:8](https://github.com/acme/widgets/blob/0123456789abcdef0123456789abcdef01234567/tests/fixtures/markdown/pair.ts#L8)

  <details>
  <summary>suppressed code</summary>

  ```typescript
  9 | export const LEGACY = widget(undefined);
  ```

  </details>

- **ruff E501** — _the vendor URL cannot be wrapped_ — [tests/fixtures/markdown/single.py:5](https://github.com/acme/widgets/blob/0123456789abcdef0123456789abcdef01234567/tests/fixtures/markdown/single.py#L5)

  <details>
  <summary>suppressed code</summary>

  ```python
  5 | CATALOGUE = urllib.request.urlopen("https://example.invalid/a/very/long/vendor/catalogue.json")  # noqa: E501  # the vendor URL cannot be wrapped
  ```

  </details>

