# unsafe Code Notes

The whole crate denies `unsafe` (`#![deny(unsafe_code)]`). The exceptions are a
few Windows APIs with no safe alternative, concentrated in a few modules with
`#[allow(unsafe_code)]`, wrapping the `unsafe` calls into safe interfaces.

Exception modules:

- `p_w::console`: Windows console adaptation.
- `p_w::acl`: named-pipe DACL setup.
- `p_w::acp`: system code page (ACP) retrieval.
- `p_w::service::u`: Windows service (SCM) integration.

Each exception module's wrapping approach and unsafe call sites are in the code
comments.
