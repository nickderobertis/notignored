# notignored-sdk (Python)

A scaffold. The project is wired into the Nx graph — `bootstrap`, `format`,
`format-check`, `lint`, `test`, `check` — and its placeholder suite proves that
wiring end to end, but no SDK surface has landed yet.

The CLI is what you want today: `pip install notignored-cli`.

## Working on it

From the repository root:

```bash
just bootstrap                             # provisions every project
just nx run notignored-sdk-python:check    # this project's gate alone
just check                                 # the whole repo's gate
```
