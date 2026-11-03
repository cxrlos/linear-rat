# Poll first, with a snapshot cache

lrat stays fresh by polling for issues whose `updatedAt` is newer than a stored watermark, and starts instantly by painting from a JSON snapshot of the last session before that first poll returns. Linear's GraphQL subscriptions (`wss://api.linear.app/graphql`, `graphql-transport-ws`) are an optional accelerator on top of polling, never a replacement: they deliver nothing while disconnected, have no replay, no delete event and no label filter, and are undocumented. So the delta poll is needed for correctness regardless.

## Considered options

- **Subscriptions only**: rejected for the gaps above.
- **SQLite cache in 0.1.0**: deferred. The snapshot can be thrown away on any mismatch, so it needs no migrations; SQLite comes when offline mode, local search or analytics history need queries.

## Consequences

- Polling spends the hourly request and complexity budgets, so queries stay small (bounded nested connections) and the poll interval backs off on the rate-limit headers.
- The snapshot holds workspace data on disk, so it is written with mode 0600 and never contains the API key.
- A personal API key authenticates the socket when sent as the `Authorization` header of the upgrade request (probed 2026-11-03), so live updates ship in 0.1.0 as an opt-in setting rather than waiting for OAuth.
