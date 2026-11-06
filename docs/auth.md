# Logging in

lrat needs a Linear personal API key, created at <https://linear.app/settings/account/security>. `lrat auth login` checks the key with Linear, then keeps it in the OS keyring under service `linear-rat` and the workspace's URL key as the account: the Secret Service (GNOME Keyring, KWallet) on Linux, the Keychain on macOS. A key Linear rejects is never stored.

There are three ways to hand it the key:

| Command | Where the key comes from |
| --- | --- |
| `lrat auth login` | A prompt on the terminal, without echo. |
| `lrat auth login --stdin` | Standard input, such as `pass show linear \| lrat auth login --stdin`. Surrounding whitespace is trimmed. |
| `lrat auth login --from-keyring [SERVICE/ACCOUNT]` | An entry already in the OS keyring. With no value it reads service `linear`, account `api-key`. |

`--stdin` and `--from-keyring` cannot be combined.

## From an existing keyring entry

`--from-keyring` reads the entry and leaves it as it is; lrat stores its own copy. The value splits at the first `/`, so `--from-keyring work/me@example.com` reads service `work`, account `me@example.com`, and an account may hold a `/`.

The entry lrat looks for is, on each system:

- Linux: a Secret Service item with the attributes `service` and `username`. To make the default entry: `secret-tool store --label='Linear API key' service linear username api-key`, then paste the key.
- macOS: a generic password in the Keychain with that service and account. To make the default entry: `security add-generic-password -s linear -a api-key -w`, then paste the key. macOS may ask whether lrat may read it.

When the entry is missing, the keyring cannot be reached, the entry does not hold a valid key or Linear rejects the key, `lrat auth login` says which and stores nothing.

## Checking and removing

`lrat auth status` names where the key in use comes from, its workspace and its viewer. `lrat auth logout` deletes lrat's own entry; an entry read with `--from-keyring` stays.

`LINEAR_API_KEY`, when set, wins over the keyring: lrat and `lrat auth status` use it, and `lrat auth login` says so after storing a key.
