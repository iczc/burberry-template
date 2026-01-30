# burberry-template

MEV bot template (Rust) built on [Burberry](https://github.com/tonyke-bot/burberry). It subscribes to blocks and mempool, builds txs by strategy, and submits bundles to Flashbots and other builders.

## Run

Set `ETH_RPC_URL` (WebSocket), `PRIVATE_KEY`, `RELAY_KEY`; strategy args: `--contract-address`, `--calldata`, and optionally `--max-priority-fee` (default 1 gwei).

```bash
cargo run -- \
  --rpc-url "$ETH_RPC_URL" \
  --private-key "$PRIVATE_KEY" \
  --relay-key "$RELAY_KEY" \
  --contract-address <CONTRACT_ADDRESS> \
  --calldata <HEX_CALLDATA>
```

## Structure

- `strategy.rs` — event handling, tx build, bundle submit
- `executor.rs` — bundle submission to multiple builders
- `block_state.rs` — block state, next-block base fee
- `simulator.rs` — tx simulation
- `quoter.rs` — V3 pools, routes, swap quotes
