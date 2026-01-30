use crate::constants::WETH_ADDRESS;
use alloy::{
    network::TransactionBuilder,
    primitives::{address, aliases::U24, Address, Bytes, U160, U256},
    providers::Provider,
    rpc::types::{BlockId, TransactionRequest},
    sol,
    sol_types::{SolCall, SolValue},
};
use eyre::eyre;
use futures::future::join_all;
use rayon::prelude::{IntoParallelRefIterator, ParallelIterator};
use serde::{de::DeserializeOwned, Deserialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
    sync::Arc,
};
use tracing::log::{debug, info};

#[derive(Deserialize, Debug)]
struct SubgraphToken {
    id: String,
}

#[derive(Deserialize, Debug)]
struct V3SubgraphPool {
    id: String,
    #[serde(rename = "feeTier")]
    fee_tier: String,
    #[allow(dead_code)]
    #[serde(default)]
    liquidity: Option<String>,
    token0: SubgraphToken,
    token1: SubgraphToken,
    #[serde(rename = "tvlETH")]
    tvl_eth: f64,
    #[allow(dead_code)]
    #[serde(rename = "tvlUSD", default)]
    tvl_usd: Option<f64>,
}

fn load_json_file<T: DeserializeOwned>(path: impl AsRef<Path>) -> T {
    let path = path.as_ref();
    let file =
        fs::File::open(path).unwrap_or_else(|e| panic!("read file {}: {}", path.display(), e));
    serde_json::from_reader(file).unwrap_or_else(|e| panic!("parse JSON {}: {}", path.display(), e))
}

const QUOTER_ADDRESS: Address = address!("0xb27308f9F90D607463bb33eA1BeBb41C27CE5AB6");
const DEFAULT_MAX_HOPS: u8 = 2;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TradeType {
    #[default]
    ExactInput,
    ExactOutput,
}

#[derive(Debug, Clone)]
pub struct Pool {
    pub address: Address,
    pub token0: Address,
    pub token1: Address,
    pub fee: u32,
}

impl Pool {
    pub fn tokens(&self) -> [Address; 2] {
        [self.token0, self.token1]
    }

    pub fn involves_token(&self, token: Address) -> bool {
        self.token0 == token || self.token1 == token
    }

    pub fn get_token_out(&self, token_in: Address) -> Address {
        if self.token0 == token_in {
            self.token1
        } else {
            self.token0
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Route {
    pub token_in: Address,
    pub token_out: Address,
    pub pools: Vec<Pool>,
}

pub struct ComputeRoutes {
    token_in: Address,
    token_out: Address,
    pools: Vec<Pool>,
    max_hops: u8,
    pools_used: Vec<bool>,
    routes: Vec<Route>,
}

impl ComputeRoutes {
    pub fn new(token_in: Address, token_out: Address, pools: Vec<Pool>, max_hops: u8) -> Self {
        let pool_len = pools.len();
        ComputeRoutes {
            token_in,
            token_out,
            pools,
            max_hops,
            pools_used: vec![false; pool_len],
            routes: vec![],
        }
    }

    pub fn get_routes(self) -> Vec<Route> {
        self.routes
    }

    // https://github.com/Uniswap/smart-order-router/blob/main/src/routers/alpha-router/functions/compute-all-routes.ts
    pub fn compute_all_univ3_routes(&mut self) {
        let mut current_route = Vec::with_capacity(self.max_hops as usize);
        self.compute_routes(&mut current_route, None);
    }

    fn compute_routes(
        &mut self,
        current_route: &mut Vec<Pool>,
        previous_token_out: Option<Address>,
    ) {
        if current_route.len() > self.max_hops as usize {
            return;
        }

        if !current_route.is_empty() && current_route.last().unwrap().involves_token(self.token_out)
        {
            self.routes.push(Route {
                token_in: self.token_in,
                token_out: self.token_out,
                pools: current_route.clone(),
            });
            return;
        }

        let previous_token_out = previous_token_out.unwrap_or(self.token_in);

        for i in 0..self.pools.len() {
            if self.pools_used[i] {
                continue;
            }

            let cur_pool = &self.pools[i];

            if !cur_pool.involves_token(previous_token_out) {
                continue;
            }

            let current_token_out = cur_pool.get_token_out(previous_token_out);

            current_route.push(cur_pool.clone());
            self.pools_used[i] = true;
            self.compute_routes(current_route, Some(current_token_out));
            self.pools_used[i] = false;
            current_route.pop();
        }
    }
}

sol!(
    #[allow(missing_docs)]
    #[sol(rpc)]
    interface IQuoter {
        function quoteExactInput(bytes memory path, uint256 amountIn) external returns (uint256 amountOut);

        function quoteExactInputSingle(
            address tokenIn,
            address tokenOut,
            uint24 fee,
            uint256 amountIn,
            uint160 sqrtPriceLimitX96
        ) external returns (uint256 amountOut);

        function quoteExactOutput(bytes memory path, uint256 amountOut) external returns (uint256 amountIn);

        function quoteExactOutputSingle(
            address tokenIn,
            address tokenOut,
            uint24 fee,
            uint256 amountOut,
            uint160 sqrtPriceLimitX96
        ) external returns (uint256 amountIn);
    }
);

pub struct UniswapV3Quoter {
    provider: Arc<dyn Provider>,
    pools: Vec<Pool>,
    tokens_to_weth_routes: HashMap<Address, Vec<Route>>,
}

impl UniswapV3Quoter {
    #[must_use]
    pub fn new(
        provider: Arc<dyn Provider>,
        pools_path: impl AsRef<Path>,
        tvl_eth_min: Option<f64>,
    ) -> Self {
        let pools: Vec<Pool> = load_json_file::<Vec<V3SubgraphPool>>(&pools_path)
            .into_iter()
            .filter_map(|pool| {
                if tvl_eth_min.is_some_and(|min_tvl| pool.tvl_eth <= min_tvl) {
                    return None;
                }
                let address = pool.id.parse().ok()?;
                let token0 = pool.token0.id.parse().ok()?;
                let token1 = pool.token1.id.parse().ok()?;
                let fee = pool.fee_tier.parse().ok()?;
                Some((
                    address,
                    Pool {
                        address,
                        token0,
                        token1,
                        fee,
                    },
                ))
            })
            .collect::<HashMap<_, _>>()
            .into_values()
            .collect();

        info!("pools loaded: {}", pools.len());

        Self {
            provider,
            pools,
            tokens_to_weth_routes: HashMap::new(),
        }
    }

    #[must_use]
    pub fn new_and_precompute_weth_routes(
        provider: Arc<dyn Provider>,
        pools_path: impl AsRef<Path>,
        tvl_eth_min: Option<f64>,
    ) -> Self {
        let mut quoter = Self::new(provider, pools_path, tvl_eth_min);
        let tokens: HashSet<Address> = quoter
            .pools
            .iter()
            .flat_map(|pool| [pool.token0, pool.token1])
            .collect();
        quoter.tokens_to_weth_routes = quoter.compute_tokens_to_weth_routes(tokens);

        let (token_count, path_count) = (
            quoter.tokens_to_weth_routes.len(),
            quoter
                .tokens_to_weth_routes
                .values()
                .map(Vec::len)
                .sum::<usize>(),
        );
        info!("token-to-weth routes: {token_count} tokens, {path_count} paths");

        quoter
    }

    fn compute_tokens_to_weth_routes(
        &self,
        tokens: impl IntoIterator<Item = Address>,
    ) -> HashMap<Address, Vec<Route>> {
        let tokens: Vec<Address> = tokens.into_iter().collect();
        tokens
            .par_iter()
            .filter(|&&token| token != WETH_ADDRESS)
            .map(|&token| {
                let routes = self.get_all_routes(token, WETH_ADDRESS, Some(DEFAULT_MAX_HOPS));
                (token, routes)
            })
            .filter(|(_, routes)| !routes.is_empty())
            .collect()
    }

    pub async fn quote_best_amount_out(
        &self,
        token_in: Address,
        token_out: Address,
        amount_in: U256,
    ) -> eyre::Result<(U256, Route)> {
        eyre::ensure!(
            token_in != token_out,
            "token_in and token_out cannot be the same"
        );

        let routes = if token_out == WETH_ADDRESS {
            self.tokens_to_weth_routes
                .get(&token_in)
                .cloned()
                .unwrap_or_else(|| {
                    debug!("route not found in cache for token {token_in:?} to WETH, computing...");
                    self.get_all_routes(token_in, WETH_ADDRESS, Some(DEFAULT_MAX_HOPS))
                })
        } else {
            self.get_all_routes(token_in, token_out, Some(DEFAULT_MAX_HOPS))
        };

        eyre::ensure!(
            !routes.is_empty(),
            "no swap route found from {token_in:?} to {token_out:?}"
        );

        let quote_futures: Vec<_> = routes
            .into_iter()
            .map(|route| {
                let provider = self.provider.clone();
                async move {
                    let calldata =
                        build_quote_calldata(amount_in, &route, TradeType::ExactInput, None);
                    call_quoter(&provider, calldata, None)
                        .await
                        .map(|amount_out| (amount_out, route))
                        .map_err(|e| {
                            debug!("route quote failed, error: {e:#}");
                            e
                        })
                        .ok()
                }
            })
            .collect();

        let (max_amount_out, best_route) = join_all(quote_futures)
            .await
            .into_iter()
            .flatten()
            .max_by_key(|(amount_out, _)| *amount_out)
            .ok_or_else(|| eyre!("no valid route found from {token_in:?} to {token_out:?}"))?;

        Ok((max_amount_out, best_route))
    }

    fn get_all_routes(
        &self,
        token_in: Address,
        token_out: Address,
        max_hops: Option<u8>,
    ) -> Vec<Route> {
        let mut compute_route = ComputeRoutes::new(
            token_in,
            token_out,
            self.pools.clone(),
            max_hops.unwrap_or(DEFAULT_MAX_HOPS),
        );
        compute_route.compute_all_univ3_routes();

        compute_route.get_routes()
    }

    pub async fn quote_single(
        &self,
        token_in: Address,
        amount: U256,
        pool: Pool,
        trade_type: Option<TradeType>,
    ) -> eyre::Result<U256> {
        let trade_type = trade_type.unwrap_or_default();
        let route = Route {
            token_in,
            token_out: pool.get_token_out(token_in),
            pools: vec![pool],
        };
        let calldata = build_quote_calldata(amount, &route, trade_type, None);
        call_quoter(&self.provider, calldata, None).await
    }
}

fn encode_leg(pool: &Pool, token_in: Address) -> (Address, Vec<u8>) {
    let token_out = pool.get_token_out(token_in);
    let leg = (token_in, U24::from(pool.fee));
    (token_out, leg.abi_encode_packed())
}

pub fn encode_route_to_path(route: &Route, exact_output: bool) -> Vec<u8> {
    let mut path: Vec<u8> = Vec::with_capacity(23 * route.pools.len() + 20);
    if exact_output {
        let mut output_token = route.token_out;
        for pool in route.pools.iter().rev() {
            let (input_token, leg) = encode_leg(pool, output_token);
            output_token = input_token;
            path.extend(leg);
        }
        path.extend(route.token_in.abi_encode_packed());
    } else {
        let mut input_token = route.token_in;
        for pool in route.pools.iter() {
            let (output_token, leg) = encode_leg(pool, input_token);
            input_token = output_token;
            path.extend(leg);
        }
        path.extend(route.token_out.abi_encode_packed());
    }
    path
}

fn build_quote_calldata(
    amount: U256,
    route: &Route,
    trade_type: TradeType,
    sqrt_price_limit_x96: Option<U160>,
) -> Bytes {
    let sqrt_price_limit = sqrt_price_limit_x96.unwrap_or_default();

    if route.pools.len() == 1 {
        let pool = route.pools.first().unwrap();
        match trade_type {
            TradeType::ExactInput => IQuoter::quoteExactInputSingleCall {
                tokenIn: route.token_in,
                tokenOut: route.token_out,
                fee: U24::from(pool.fee),
                amountIn: amount,
                sqrtPriceLimitX96: sqrt_price_limit,
            }
            .abi_encode()
            .into(),
            TradeType::ExactOutput => IQuoter::quoteExactOutputSingleCall {
                tokenIn: route.token_in,
                tokenOut: route.token_out,
                fee: U24::from(pool.fee),
                amountOut: amount,
                sqrtPriceLimitX96: sqrt_price_limit,
            }
            .abi_encode()
            .into(),
        }
    } else {
        let path = encode_route_to_path(route, trade_type == TradeType::ExactOutput);
        match trade_type {
            TradeType::ExactInput => IQuoter::quoteExactInputCall {
                path: path.into(),
                amountIn: amount,
            }
            .abi_encode()
            .into(),
            TradeType::ExactOutput => IQuoter::quoteExactOutputCall {
                path: path.into(),
                amountOut: amount,
            }
            .abi_encode()
            .into(),
        }
    }
}

async fn call_quoter(
    provider: &Arc<dyn Provider>,
    calldata: Bytes,
    block: Option<BlockId>,
) -> eyre::Result<U256> {
    let tx = TransactionRequest::default()
        .with_to(QUOTER_ADDRESS)
        .with_input(calldata);

    let call = provider.root().call(tx);
    let res = if let Some(block) = block {
        call.block(block).await?
    } else {
        call.await?
    };

    U256::abi_decode(&res).map_err(|e| eyre!("failed to decode quoter return data: {e:#}"))
}

#[cfg(test)]
mod tests {
    use crate::quoter::UniswapV3Quoter;
    use alloy::primitives::utils::parse_units;
    use alloy::primitives::{address, Address, U256};
    use alloy::providers::ProviderBuilder;
    use std::path::PathBuf;
    use std::sync::Arc;

    const RPC_URL: &str = "https://eth.merkle.io";
    const WETH: Address = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
    const USDC: Address = address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");

    fn pools_fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pools.json")
    }

    #[tokio::test]
    async fn test_get_all_univ3_routes() {
        let provider = ProviderBuilder::new().connect_http(RPC_URL.parse().unwrap());
        let quoter = UniswapV3Quoter::new(Arc::new(provider), pools_fixture_path(), None);

        let routes = quoter.get_all_routes(USDC, WETH, Some(2));
        assert!(!routes.is_empty(), "Should find at least one route");
        for route in routes {
            let token_in = route.token_in;
            let token_out = route.token_out;
            assert_eq!(token_in, USDC);
            assert_eq!(token_out, WETH);
            let hop = route.pools.len();
            assert!(hop <= 2, "Hop count should be at most 2");
            if hop == 1 {
                assert!(route.pools[0].involves_token(token_in));
                assert!(route.pools[0].involves_token(token_out));
            } else if hop == 2 {
                assert!(route.pools[0].involves_token(token_in));
                assert!(route.pools[1].involves_token(token_out));
            } else {
                panic!("hop should be less than 3");
            }
        }
    }

    #[tokio::test]
    async fn test_quote_best_amount_out() {
        let provider = ProviderBuilder::new().connect_http(RPC_URL.parse().unwrap());
        let quoter = UniswapV3Quoter::new(Arc::new(provider), pools_fixture_path(), None);
        let one_usdc: U256 = parse_units("1.0", "mwei").unwrap().into();
        let (amount_out, route) = quoter
            .quote_best_amount_out(USDC, WETH, one_usdc)
            .await
            .unwrap();
        assert_ne!(amount_out, U256::default());
        assert_eq!(route.pools.is_empty(), false);
    }

    #[test]
    fn test_precompute_tokens_to_weth_routes() {
        use crate::constants::WETH_ADDRESS;

        let provider = ProviderBuilder::new().connect_http(RPC_URL.parse().unwrap());
        let quoter = UniswapV3Quoter::new_and_precompute_weth_routes(
            Arc::new(provider),
            pools_fixture_path(),
            None,
        );

        assert!(
            quoter.tokens_to_weth_routes.len() > 0,
            "Should cache at least some routes"
        );

        assert!(
            !quoter.tokens_to_weth_routes.contains_key(&WETH_ADDRESS),
            "WETH should not be in the cache"
        );
        assert!(
            quoter.tokens_to_weth_routes.contains_key(&USDC),
            "USDC should be in the cache"
        );
    }
}
