use alloy::{
    providers::ext::MevApi, providers::ProviderBuilder, rpc::types::mev::EthSendBundle,
    signers::local::PrivateKeySigner,
};
use eyre::{eyre, Result};
use std::sync::Arc;
use tracing::{debug, error};
use url::Url;

#[derive(Clone)]
struct FlashbotsRelay {
    url: String,
    signer: Option<PrivateKeySigner>,
    name: String,
}

impl FlashbotsRelay {
    fn new(url: &str, signer: Option<PrivateKeySigner>) -> Result<Self> {
        Ok(FlashbotsRelay {
            url: url.to_string(),
            signer,
            name: url.to_string(),
        })
    }

    async fn send_bundle(&self, bundle: EthSendBundle) -> Result<()> {
        let provider = ProviderBuilder::new().connect_http(Url::parse(&self.url)?);

        let result = match self.signer.clone() {
            Some(signer) => provider.send_bundle(bundle).with_auth(signer).await,
            None => provider.send_bundle(bundle).await,
        };

        match result {
            Ok(_resp) => {
                debug!("Bundle sent to: {}", self.name);
                Ok(())
            }
            Err(error) => {
                error!("{} {}", self.name, error.to_string());
                Err(eyre!("FLASHBOTS_RELAY_ERROR"))
            }
        }
    }
}

pub struct FlashbotsBroadcaster {
    relays: Arc<Vec<FlashbotsRelay>>,
    signer: Arc<PrivateKeySigner>,
}

impl FlashbotsBroadcaster {
    pub fn new(signer: Option<PrivateKeySigner>) -> Result<Self> {
        let signer = Arc::new(signer.unwrap_or(PrivateKeySigner::random()));
        let relays = Arc::new(Vec::new());

        Ok(FlashbotsBroadcaster { relays, signer })
    }

    pub fn with_default_relays(mut self) -> Result<Self> {
        let relay_configs = vec![
            ("https://relay.flashbots.net", false),
            ("https://rpc.beaverbuild.org/", false),
            ("https://rpc.titanbuilder.xyz", false),
            ("https://rsync-builder.xyz", false),
            ("https://api.edennetwork.io/v1/bundle", false),
            ("https://eth-builder.com", true),
            ("https://api.securerpc.com/v1", true),
            ("https://BuildAI.net", true),
            ("https://rpc.payload.de", true),
            ("https://rpc.f1b.io", false),
            ("https://rpc.lokibuilder.xyz", false),
            ("https://rpc.ibuilder.xyz", false),
            ("https://rpc.jetbldr.xyz", false),
            ("https://rpc.penguinbuild.org", false),
            ("https://builder.gmbit.co/rpc", false),
        ];

        let mut relays = Vec::new();
        for (url, no_sign) in relay_configs {
            let signer = if no_sign {
                None
            } else {
                Some((*self.signer).clone())
            };
            relays.push(FlashbotsRelay::new(url, signer)?);
        }

        self.relays = Arc::new(relays);
        Ok(self)
    }

    pub fn with_relay(mut self, url: &str) -> Result<Self> {
        let mut relays = Arc::try_unwrap(self.relays).unwrap_or_else(|arc| (*arc).clone());

        relays.push(FlashbotsRelay::new(url, Some((*self.signer).clone()))?);
        self.relays = Arc::new(relays);
        Ok(self)
    }

    pub async fn broadcast_bundle(&self, bundle: EthSendBundle) -> Result<()> {
        for relay in self.relays.iter() {
            let relay_clone = relay.clone();
            let bundle_clone = bundle.clone();

            tokio::spawn(async move {
                debug!("Sending bundles to {}", relay_clone.name);

                if let Err(e) = relay_clone.send_bundle(bundle_clone).await {
                    error!("Error sending bundle to {}: {}", relay_clone.name, e);
                }
            });
        }

        Ok(())
    }
}
