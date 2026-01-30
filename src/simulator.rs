use crate::constants::WETH_ADDRESS;
use alloy::{
    primitives::{Address, Log, I256, U256},
    providers::Provider,
    rpc::types::{
        simulate::{SimBlock, SimCallResult, SimulatePayload},
        BlockOverrides, TransactionRequest,
    },
    sol,
    sol_types::SolEvent,
};
use eyre::eyre;
use std::{collections::HashMap, sync::Arc};

pub type BalanceChanges = HashMap<Address, HashMap<Address, I256>>;

sol! {
    event Transfer(address indexed from, address indexed to, uint256 value);
    event Deposit(address indexed dst, uint256 wad);
    event Withdrawal(address indexed src, uint256 wad);
}

pub async fn simulate_tx(
    provider: Arc<dyn Provider>,
    tx: TransactionRequest,
    block_overrides: Option<BlockOverrides>,
) -> eyre::Result<(BalanceChanges, SimCallResult)> {
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

    let simulated_blocks = provider.simulate(&simulate_payload).await?;
    let last_call = simulated_blocks
        .last()
        .and_then(|block| block.calls.last())
        .ok_or_else(|| eyre!("failed to get last call from simulation result"))?;

    let balance_changes = calculate_erc20_balance_changes(last_call);

    Ok((balance_changes, last_call.clone()))
}

pub fn calculate_erc20_balance_changes(call: &SimCallResult) -> BalanceChanges {
    // If transaction failed or no logs, return empty balance changes
    if !call.status || call.logs.is_empty() {
        return HashMap::new();
    }

    let mut balance_changes = HashMap::new();
    handle_logs(&call.logs, &mut balance_changes);

    balance_changes
}

fn handle_logs(logs: &[alloy::rpc::types::Log], balance_changes: &mut BalanceChanges) {
    for log in logs {
        let token = log.address();
        let Some(alloy_log) = Log::new(token, log.topics().into(), log.data().data.clone()) else {
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
            continue;
        }

        // Handle WETH-specific events
        if token == WETH_ADDRESS {
            // Handle Deposit events (WETH)
            if let Ok(deposit) = Deposit::decode_log(&alloy_log) {
                update_balance_changes(
                    balance_changes,
                    WETH_ADDRESS,
                    deposit.dst,
                    token,
                    deposit.wad,
                );
            }
            // Handle Withdrawal events (WETH)
            else if let Ok(withdrawal) = Withdrawal::decode_log(&alloy_log) {
                update_balance_changes(
                    balance_changes,
                    withdrawal.src,
                    WETH_ADDRESS,
                    token,
                    withdrawal.wad,
                );
            }
        }
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
