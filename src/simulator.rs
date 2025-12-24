use alloy::{
    primitives::{address, Address, Log, I256, U256},
    providers::Provider,
    rpc::types::{
        simulate::{SimBlock, SimulatePayload, SimulatedBlock},
        Block, BlockOverrides, TransactionRequest,
    },
    sol,
    sol_types::SolEvent,
};
use std::{collections::HashMap, sync::Arc};

type BalanceChanges = HashMap<Address, HashMap<Address, I256>>;

// WETH address on Ethereum mainnet
const WETH_ADDRESS: Address = address!("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");

sol! {
    event Transfer(address indexed from, address indexed to, uint256 value);
    event Deposit(address indexed dst, uint256 wad);
    event Withdrawal(address indexed src, uint256 wad);
}

/// Simulates a transaction and returns the simulated blocks.
pub async fn simulate_transaction(
    provider: Arc<dyn Provider>,
    tx: TransactionRequest,
    block_overrides: Option<BlockOverrides>,
) -> eyre::Result<Vec<SimulatedBlock<Block>>> {
    let simulate_payload = SimulatePayload {
        block_state_calls: vec![SimBlock {
            block_overrides,
            state_overrides: None,
            calls: vec![tx],
        }],
        trace_transfers: true,
        validation: false,
        return_full_transactions: false,
    };

    Ok(provider.simulate(&simulate_payload).await?)
}

/// Calculates ERC20 token balance changes from simulated blocks.
pub fn calculate_erc20_balance_changes(
    simulated_blocks: &[SimulatedBlock<Block>],
) -> BalanceChanges {
    let mut balance_changes = HashMap::new();

    for block in simulated_blocks {
        for call_result in &block.calls {
            handle_transfer_events(&call_result.logs, &mut balance_changes);
        }
    }

    balance_changes
}

fn handle_transfer_events(logs: &[alloy::rpc::types::Log], balance_changes: &mut BalanceChanges) {
    for log in logs {
        let token = log.address();
        let Some(alloy_log) = Log::new(token, log.topics().to_vec(), log.data().data.clone())
        else {
            continue;
        };

        // Handle Transfer events
        if let Ok(transfer) = Transfer::decode_log(&alloy_log) {
            update_balance_changes(
                balance_changes,
                transfer.from,
                transfer.to,
                token,
                transfer.value,
            );
        }
        // Handle WETH-specific events
        else if token == WETH_ADDRESS {
            handle_weth_events(&alloy_log, balance_changes);
        }
    }
}

fn handle_weth_events(alloy_log: &Log, balance_changes: &mut BalanceChanges) {
    // Handle Deposit events (WETH)
    if let Ok(deposit) = Deposit::decode_log(alloy_log) {
        update_balance_changes(
            balance_changes,
            WETH_ADDRESS,
            deposit.dst,
            WETH_ADDRESS,
            deposit.wad,
        );
    }
    // Handle Withdrawal events (WETH)
    else if let Ok(withdrawal) = Withdrawal::decode_log(alloy_log) {
        update_balance_changes(
            balance_changes,
            withdrawal.src,
            WETH_ADDRESS,
            WETH_ADDRESS,
            withdrawal.wad,
        );
    }
}

fn update_balance_changes(
    balance_changes: &mut BalanceChanges,
    from: Address,
    to: Address,
    token: Address,
    value: U256,
) {
    let change = I256::from_raw(value);

    *balance_changes
        .entry(from)
        .or_default()
        .entry(token)
        .or_default() -= change;

    *balance_changes
        .entry(to)
        .or_default()
        .entry(token)
        .or_default() += change;
}
