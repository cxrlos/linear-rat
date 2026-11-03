# Hand-written GraphQL over an SDK

lrat talks to Linear through its own small client: `.graphql` query files with serde types, reqwest underneath, and a test that validates every query against a pinned copy of Linear's schema. I evaluated lineark-sdk 4.0.1, the typed Rust SDK for Linear, and rejected it: it drops the response headers that carry the request and complexity budgets, has no timeouts or client injection, discards partial data on any GraphQL error, cannot pass arguments to nested connections (so cheap list queries need raw strings anyway), added about 35 s to every cold build in my benchmark, and has shipped four major versions in eight months under one maintainer.

## Considered options

- **lineark-sdk**: rejected for the reasons above. It also has no websocket support, which was the main reason to consider it.
- **cynic or graphql_client codegen**: rejected. cynic's own docs warn about compile times on schemas this size (1.39 MB), and graphql_client does not support reqwest 0.13.

## Consequences

I write and maintain about 20 queries and the input types for the filters lrat uses. Schema drift is caught by the validation test, not the compiler. Live updates need their own websocket client whichever option was chosen.
