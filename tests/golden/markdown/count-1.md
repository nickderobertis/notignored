<!-- notignored-report -->

### notignored: 1 suppression

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

