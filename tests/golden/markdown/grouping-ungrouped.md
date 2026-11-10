<!-- notignored-report -->

### notignored: 15 suppressions

- **rust dead\_code** — _kept for the plugin API_ — [tests/fixtures/grouping/src/lib.rs:4](https://github.com/acme/widgets/blob/0123456789abcdef0123456789abcdef01234567/tests/fixtures/grouping/src/lib.rs#L4)

  <details>
  <summary>suppressed code</summary>

  ```rust
  4 | #[allow(dead_code, reason = "kept for the plugin API")]
  5 | fn register() {}
  ```

  </details>

- **rust dead\_code** — _kept for the plugin API_ — [tests/fixtures/grouping/src/lib.rs:7](https://github.com/acme/widgets/blob/0123456789abcdef0123456789abcdef01234567/tests/fixtures/grouping/src/lib.rs#L7)

  <details>
  <summary>suppressed code</summary>

  ```rust
   7 | #[allow( dead_code , reason = "kept for the plugin API" )]
   8 | fn unregister() {
   9 |     let _ = register;
  10 | }
  ```

  </details>

- **ruff E501, F401** — _re-exported for the plugin API_ — [tests/fixtures/grouping/src/models.py:3](https://github.com/acme/widgets/blob/0123456789abcdef0123456789abcdef01234567/tests/fixtures/grouping/src/models.py#L3)

  <details>
  <summary>suppressed code</summary>

  ```python
    1 | """Grouping fixture: directives that test what makes two suppressions identical."""
    2 |
  > 3 | import json  # noqa: E501, F401  # re-exported for the plugin API
    4 | import os  # noqa: F401,E501  # re-exported for the plugin API
    5 | import re  # noqa:E501,F401,E501   #   re-exported   for the plugin API
  ```

  </details>

- **ruff F401, E501** — _re-exported for the plugin API_ — [tests/fixtures/grouping/src/models.py:4](https://github.com/acme/widgets/blob/0123456789abcdef0123456789abcdef01234567/tests/fixtures/grouping/src/models.py#L4)

  <details>
  <summary>suppressed code</summary>

  ```python
    2 |
    3 | import json  # noqa: E501, F401  # re-exported for the plugin API
  > 4 | import os  # noqa: F401,E501  # re-exported for the plugin API
    5 | import re  # noqa:E501,F401,E501   #   re-exported   for the plugin API
    6 | import sys  # noqa  # re-exported for the plugin API
  ```

  </details>

- **ruff E501, F401, E501** — _re-exported for the plugin API_ — [tests/fixtures/grouping/src/models.py:5](https://github.com/acme/widgets/blob/0123456789abcdef0123456789abcdef01234567/tests/fixtures/grouping/src/models.py#L5)

  <details>
  <summary>suppressed code</summary>

  ```python
    3 | import json  # noqa: E501, F401  # re-exported for the plugin API
    4 | import os  # noqa: F401,E501  # re-exported for the plugin API
  > 5 | import re  # noqa:E501,F401,E501   #   re-exported   for the plugin API
    6 | import sys  # noqa  # re-exported for the plugin API
    7 | import io  # noqa: F401  # re-exported for the plugin API
  ```

  </details>

- **ruff (all rules)** — _re-exported for the plugin API_ — [tests/fixtures/grouping/src/models.py:6](https://github.com/acme/widgets/blob/0123456789abcdef0123456789abcdef01234567/tests/fixtures/grouping/src/models.py#L6)

  <details>
  <summary>suppressed code</summary>

  ```python
    4 | import os  # noqa: F401,E501  # re-exported for the plugin API
    5 | import re  # noqa:E501,F401,E501   #   re-exported   for the plugin API
  > 6 | import sys  # noqa  # re-exported for the plugin API
    7 | import io  # noqa: F401  # re-exported for the plugin API
    8 | import abc  # noqa: e501, F401  # re-exported for the plugin API
  ```

  </details>

- **ruff F401** — _re-exported for the plugin API_ — [tests/fixtures/grouping/src/models.py:7](https://github.com/acme/widgets/blob/0123456789abcdef0123456789abcdef01234567/tests/fixtures/grouping/src/models.py#L7)

  <details>
  <summary>suppressed code</summary>

  ```python
    5 | import re  # noqa:E501,F401,E501   #   re-exported   for the plugin API
    6 | import sys  # noqa  # re-exported for the plugin API
  > 7 | import io  # noqa: F401  # re-exported for the plugin API
    8 | import abc  # noqa: e501, F401  # re-exported for the plugin API
    9 | import csv  # noqa: E501, F401  # re-exported for the public API
  ```

  </details>

- **ruff e501, F401** — _re-exported for the plugin API_ — [tests/fixtures/grouping/src/models.py:8](https://github.com/acme/widgets/blob/0123456789abcdef0123456789abcdef01234567/tests/fixtures/grouping/src/models.py#L8)

  <details>
  <summary>suppressed code</summary>

  ```python
     6 | import sys  # noqa  # re-exported for the plugin API
     7 | import io  # noqa: F401  # re-exported for the plugin API
  >  8 | import abc  # noqa: e501, F401  # re-exported for the plugin API
     9 | import csv  # noqa: E501, F401  # re-exported for the public API
    10 | # ruff: noqa: E501, F401  # re-exported for the plugin API
  ```

  </details>

- **ruff E501, F401** — _re-exported for the public API_ — [tests/fixtures/grouping/src/models.py:9](https://github.com/acme/widgets/blob/0123456789abcdef0123456789abcdef01234567/tests/fixtures/grouping/src/models.py#L9)

  <details>
  <summary>suppressed code</summary>

  ```python
     7 | import io  # noqa: F401  # re-exported for the plugin API
     8 | import abc  # noqa: e501, F401  # re-exported for the plugin API
  >  9 | import csv  # noqa: E501, F401  # re-exported for the public API
    10 | # ruff: noqa: E501, F401  # re-exported for the plugin API
    11 |
  ```

  </details>

- **ruff E501, F401** — _re-exported for the plugin API_ — [tests/fixtures/grouping/src/models.py:10](https://github.com/acme/widgets/blob/0123456789abcdef0123456789abcdef01234567/tests/fixtures/grouping/src/models.py#L10)

  <details>
  <summary>suppressed code</summary>

  _the whole file is suppressed; showing its first 10 of 13 lines._

  ```python
   1 | """Grouping fixture: directives that test what makes two suppressions identical."""
   2 |
   3 | import json  # noqa: E501, F401  # re-exported for the plugin API
   4 | import os  # noqa: F401,E501  # re-exported for the plugin API
   5 | import re  # noqa:E501,F401,E501   #   re-exported   for the plugin API
   6 | import sys  # noqa  # re-exported for the plugin API
   7 | import io  # noqa: F401  # re-exported for the plugin API
   8 | import abc  # noqa: e501, F401  # re-exported for the plugin API
   9 | import csv  # noqa: E501, F401  # re-exported for the public API
  10 | # ruff: noqa: E501, F401  # re-exported for the plugin API
  ```

  </details>

- **mypy attr-defined** — _the stub predates this constant_ — [tests/fixtures/grouping/src/models.py:12](https://github.com/acme/widgets/blob/0123456789abcdef0123456789abcdef01234567/tests/fixtures/grouping/src/models.py#L12)

  <details>
  <summary>suppressed code</summary>

  ```python
    10 | # ruff: noqa: E501, F401  # re-exported for the plugin API
    11 |
  > 12 | LIMIT = json.MAX  # type: ignore[attr-defined]  # the stub predates this constant
    13 | DEPTH = json.DEPTH  # ty: ignore[attr-defined]  # the stub predates this constant
  ```

  </details>

- **ty attr-defined** — _the stub predates this constant_ — [tests/fixtures/grouping/src/models.py:13](https://github.com/acme/widgets/blob/0123456789abcdef0123456789abcdef01234567/tests/fixtures/grouping/src/models.py#L13)

  <details>
  <summary>suppressed code</summary>

  ```python
    11 |
    12 | LIMIT = json.MAX  # type: ignore[attr-defined]  # the stub predates this constant
  > 13 | DEPTH = json.DEPTH  # ty: ignore[attr-defined]  # the stub predates this constant
  ```

  </details>

- **ruff F401, E501** — _re-exported for the plugin API_ — [tests/fixtures/grouping/src/views.py:3](https://github.com/acme/widgets/blob/0123456789abcdef0123456789abcdef01234567/tests/fixtures/grouping/src/views.py#L3)

  <details>
  <summary>suppressed code</summary>

  ```python
    1 | """Grouping fixture: the same decisions again, in a second file."""
    2 |
  > 3 | from . import models  # noqa: F401, E501  # re-exported for the plugin API
    4 |
    5 | WIDTH = models.json.WIDTH  # type: ignore[attr-defined]  # the stub predates this constant
  ```

  </details>

- **mypy attr-defined** — _the stub predates this constant_ — [tests/fixtures/grouping/src/views.py:5](https://github.com/acme/widgets/blob/0123456789abcdef0123456789abcdef01234567/tests/fixtures/grouping/src/views.py#L5)

  <details>
  <summary>suppressed code</summary>

  ```python
    3 | from . import models  # noqa: F401, E501  # re-exported for the plugin API
    4 |
  > 5 | WIDTH = models.json.WIDTH  # type: ignore[attr-defined]  # the stub predates this constant
    6 | _cache = models._cache  # pyright: ignore[reportPrivateUsage]  # the private cache is ours
  ```

  </details>

- **pyright reportPrivateUsage** — _the private cache is ours_ — [tests/fixtures/grouping/src/views.py:6](https://github.com/acme/widgets/blob/0123456789abcdef0123456789abcdef01234567/tests/fixtures/grouping/src/views.py#L6)

  <details>
  <summary>suppressed code</summary>

  ```python
    4 |
    5 | WIDTH = models.json.WIDTH  # type: ignore[attr-defined]  # the stub predates this constant
  > 6 | _cache = models._cache  # pyright: ignore[reportPrivateUsage]  # the private cache is ours
  ```

  </details>

---

<sub>Suppressions as of [`0123456`](https://github.com/acme/widgets/commit/0123456789abcdef0123456789abcdef01234567).</sub>
