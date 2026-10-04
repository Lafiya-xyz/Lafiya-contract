# Authorization decoder fixtures

`*.txt` files are golden outputs of `lafiya-cli auth decode` for the entries
built in `crates/lafiya-cli/src/auth_decode.rs` tests. Regenerate them after an
intentional format change with:

```sh
UPDATE_GOLDEN=1 cargo test -p lafiya-cli auth_decode
```

and review the diff before committing.

## Payload hash fixture

`payload_hash_fixture.json` was produced independently of the decoder with
`stellar-cli` 28.0.0, from `payload_hash_entry.json` and
`payload_hash_preimage.json` (the preimage's `network_id` is the SHA-256 of the
testnet passphrase):

```sh
stellar xdr encode --type SorobanAuthorizationEntry payload_hash_entry.json
stellar xdr encode --type HashIdPreimage payload_hash_preimage.json | base64 -d | sha256sum
```
