<!-- notignored-report -->

### notignored: 3 suppressions

- **eslint no-console** — _the banner is this program's output_ — [tests/fixtures/markdown/pair.ts:1](https://github.com/acme/widgets/blob/0123456789abcdef0123456789abcdef01234567/tests/fixtures/markdown/pair.ts#L1)

  ```typescript
  1 | // eslint-disable-next-line no-console -- the banner is this program's output
  2 | console.log("notignored");
  3 |
  ```

- **typescript (all rules)** — _no reason given_ — [tests/fixtures/markdown/pair.ts:8](https://github.com/acme/widgets/blob/0123456789abcdef0123456789abcdef01234567/tests/fixtures/markdown/pair.ts#L8)

  ```typescript
   6 | }
   7 |
   8 | // @ts-ignore
   9 | export const LEGACY = widget(undefined);
  10 |
  ```

- **ruff E501** — _the vendor URL cannot be wrapped_ — [tests/fixtures/markdown/single.py:5](https://github.com/acme/widgets/blob/0123456789abcdef0123456789abcdef01234567/tests/fixtures/markdown/single.py#L5)

  ```python
  3 | import urllib.request
  4 |
  5 | CATALOGUE = urllib.request.urlopen("https://example.invalid/a/very/long/vendor/catalogue.json")  # noqa: E501  # the vendor URL cannot be wrapped
  6 |
  7 |
  ```

