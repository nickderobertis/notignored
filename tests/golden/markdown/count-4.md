<!-- notignored-report -->

### notignored: 4 suppressions

- **ruff (all rules)** — _no reason given_ — [tests/fixtures/markdown/blanket.py:1](https://github.com/acme/widgets/blob/0123456789abcdef0123456789abcdef01234567/tests/fixtures/markdown/blanket.py#L1)

  <details>
  <summary>suppressed code</summary>

  _the whole file is suppressed._

  ```python
   1 | # ruff: noqa
   2 |
   3 | VENDORED_TABLE = {
   4 |     "a": 1,
   5 |     "b": 2,
   6 | }
   7 |
   8 | # llmlint: ignore-file[suppressions_justified] fixture input, not production code:
   9 | # the reason-less file-wide directive on line 1 is what proves a blanket
  10 | # suppression renders as "(all rules)" (tests/golden/markdown/count-4.md).
  ```

  </details>

- **eslint no-console** — _the banner is this program's output_ — [tests/fixtures/markdown/pair.ts:1](https://github.com/acme/widgets/blob/0123456789abcdef0123456789abcdef01234567/tests/fixtures/markdown/pair.ts#L1)

  <details>
  <summary>suppressed code</summary>

  ```typescript
    1 | // eslint-disable-next-line no-console -- the banner is this program's output
  > 2 | console.log("notignored");
    3 |
    4 | export function widget(name: string): string {
  ```

  </details>

- **typescript (all rules)** — _no reason given_ — [tests/fixtures/markdown/pair.ts:8](https://github.com/acme/widgets/blob/0123456789abcdef0123456789abcdef01234567/tests/fixtures/markdown/pair.ts#L8)

  <details>
  <summary>suppressed code</summary>

  ```typescript
     7 |
     8 | // @ts-ignore
  >  9 | export const LEGACY = widget(undefined);
    10 |
    11 | // llmlint: ignore-file[suppressions_justified] fixture input, not production code:
  ```

  </details>

- **ruff E501** — _the vendor URL cannot be wrapped_ — [tests/fixtures/markdown/single.py:5](https://github.com/acme/widgets/blob/0123456789abcdef0123456789abcdef01234567/tests/fixtures/markdown/single.py#L5)

  <details>
  <summary>suppressed code</summary>

  ```python
    3 | import urllib.request
    4 |
  > 5 | CATALOGUE = urllib.request.urlopen("https://example.invalid/a/very/long/vendor/catalogue.json")  # noqa: E501  # the vendor URL cannot be wrapped
    6 |
    7 |
  ```

  </details>

---

<sub>Suppressions as of [`0123456`](https://github.com/acme/widgets/commit/0123456789abcdef0123456789abcdef01234567).</sub>
