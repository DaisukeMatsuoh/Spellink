## What and why

## Checked
- [ ] Tests added or changed, and they pass (`cargo test`, `npx jest`)
- [ ] If this changes a state machine: the table in `docs/spec/` is changed first and `npm run spec` was run
- [ ] A value that crosses a trust boundary is checked where it is received (`docs/spec/interfaces.md`)
- [ ] No secret, token, address or personal detail in the change, the tests or the logs
- [ ] A decision that others will want to know is in an ADR (`docs/decisions/`)
