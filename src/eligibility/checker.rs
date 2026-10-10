use crate::eligibility::mintgo::MintGoClient;
use crate::eligibility::opensea::OpenSeaClient;
use crate::eligibility::seadrop::{SeaDropInspector, SEADROP_V1_ADDRESS};
use crate::eligibility::types::{
    ComprehensiveDropReport, ResolvedDropTarget, WalletEligibilityStatus,
};

pub struct EligibilityChecker {
    opensea: OpenSeaClient,
    mintgo: MintGoClient,
}

impl Default for EligibilityChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl EligibilityChecker {
    pub fn new() -> Self {
        Self {
            opensea: OpenSeaClient::new(),
            mintgo: MintGoClient::new(),
        }
    }

    /// Evaluates eligibility for any contract address or OpenSea link against a target wallet.
    pub async fn analyze_drop(
        &self,
        input: &str,
        wallet_address: &str,
        rpc_url: &str,
        default_chain_id: u64,
    ) -> Result<ComprehensiveDropReport, String> {
        let trimmed = input.trim();

        // 1. Resolve Target (OpenSea URL or Direct Contract Address)
        let target: ResolvedDropTarget = if let Some(slug) = OpenSeaClient::extract_slug(trimmed) {
            self.opensea.fetch_collection_by_slug(&slug).await?
        } else if trimmed.starts_with("0x") && trimmed.len() == 42 {
            let chain_slug = crate::eligibility::opensea::chain_id_to_slug(default_chain_id);
            self.opensea.fetch_collection_by_contract(chain_slug, trimmed).await
                .unwrap_or_else(|_| ResolvedDropTarget {
                    contract_address: trimmed.to_lowercase(),
                    collection_name: format!("Contract {}", &trimmed[..8]),
                    collection_slug: None,
                    chain: chain_slug.to_string(),
                    chain_id: default_chain_id,
                    opensea_url: None,
                    fee_recipient: None,
                })
        } else {
            return Err("Invalid input: must be an OpenSea collection URL or a 42-character 0x contract address.".to_string());
        };

        let seadrop_inspector = SeaDropInspector::new(rpc_url);

        // 2. Fetch MintGo drop intelligence & stages
        let mintgo_intel = self.mintgo.fetch_collection_intel(&target.chain, &target.contract_address).await.ok();
        let stages = mintgo_intel.as_ref().map(MintGoClient::parse_stages).unwrap_or_default();

        let (intel_minted, intel_max) = if let Some(ref val) = mintgo_intel {
            let m = val.get("mintedSupply").and_then(|v| v.as_u64());
            let max = val.get("maxSupply").and_then(|v| v.as_u64());
            (m, max)
        } else {
            (None, None)
        };

        // 3. Inspect On-Chain SeaDrop and Contract state
        let seadrop_public = seadrop_inspector.fetch_public_drop(&target.contract_address).await;
        let onchain_stats = seadrop_inspector.fetch_nft_mint_stats(&target.contract_address, wallet_address).await;
        let (onchain_total, onchain_max) = seadrop_inspector.fetch_supplies(&target.contract_address).await;

        let minted_supply = onchain_stats.map(|(_, cur, _)| cur)
            .or(onchain_total)
            .or(intel_minted)
            .unwrap_or(0);

        let max_supply = onchain_stats.map(|(_, _, max)| max)
            .or(onchain_max)
            .or(intel_max);

        let is_sold_out = max_supply.is_some_and(|max| max > 0 && minted_supply >= max);

        let drop_mechanism = if seadrop_public.is_some() {
            "SeaDrop v1.0".to_string()
        } else {
            "Native Contract / Custom Drop".to_string()
        };

        // 4. Calculate Wallet Eligibility
        let minted_by_wallet = onchain_stats.map(|(w, _, _)| w).unwrap_or(0);
        let max_per_wallet = seadrop_public.as_ref().map(|sd| sd.max_per_wallet).unwrap_or(20);
        let price_wei = seadrop_public.as_ref().map(|sd| sd.mint_price_wei)
            .or_else(|| stages.iter().find(|s| s.is_active_now).map(|s| s.price_wei))
            .unwrap_or(0);
        let price_eth = price_wei as f64 / 1e18;

        let remaining = if is_sold_out {
            0
        } else {
            max_per_wallet.saturating_sub(minted_by_wallet)
        };

        // 5. On-Chain Simulation Verification
        let (is_eligible, status_reason, simulated_gas) = if is_sold_out {
            (false, "Collection is 100% Sold Out".to_string(), None)
        } else if let Some(ref sd) = seadrop_public {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
            if now < sd.start_time {
                (false, format!("Public Drop is Not Active Yet (Starts in {} minutes)", (sd.start_time - now) / 60), None)
            } else if now > sd.end_time {
                (false, "Public Drop has Expired".to_string(), None)
            } else if remaining == 0 {
                (false, format!("Wallet Reached Limit ({minted_by_wallet}/{max_per_wallet})"), None)
            } else {
                // Simulate SeaDrop mintPublic on-chain
                let clean_nft = target.contract_address.trim_start_matches("0x").to_lowercase();
                let clean_fee = sd.fee_recipient.as_deref()
                    .or(target.fee_recipient.as_deref())
                    .unwrap_or("0x0000a26b00c1f0df003000390027140000faa719")
                    .trim_start_matches("0x")
                    .to_lowercase();

                let calldata = format!("0x161ac21f{:0>64}{:0>64}{:0>64}{:0>64}", clean_nft, clean_fee, "0", "1");
                let val_hex = format!("0x{:x}", sd.mint_price_wei);

                match seadrop_inspector.simulate_mint(wallet_address, SEADROP_V1_ADDRESS, &calldata, &val_hex).await {
                    Ok(gas) => (true, "Eligible to Mint Public Drop".to_string(), Some(gas)),
                    Err(revert_msg) => (false, format!("Simulation Reverted: {revert_msg}"), None),
                }
            }
        } else {
            // General NFT contract simulation: test standard function selectors
            let val_hex = format!("0x{:x}", price_wei);
            let candidates = [
                format!("0xa0712d68{:0>64}", "1"), // mint(uint256)
                "0x1249c58b".to_string(),          // mint()
                "0x4e71d92d".to_string(),          // claim()
                "0xa6f2ae3a".to_string(),          // publicMint()
            ];

            let mut sim_success = None;
            let mut last_revert = String::new();

            for calldata in &candidates {
                match seadrop_inspector.simulate_mint(wallet_address, &target.contract_address, calldata, &val_hex).await {
                    Ok(gas) => {
                        sim_success = Some(gas);
                        break;
                    }
                    Err(e) => {
                        last_revert = e;
                    }
                }
            }

            match sim_success {
                Some(gas) => (true, "Eligible to Mint via Direct Contract".to_string(), Some(gas)),
                None => {
                    // Check MintGo live mint-tx endpoint as secondary signal
                    if let Ok(tx_val) = self.mintgo.check_mint_tx(&target.chain, &target.contract_address, wallet_address, 1).await {
                        if let Some(err) = tx_val.get("error").and_then(|e| e.as_str()) {
                            (false, format!("MintGo Guard: {err}"), None)
                        } else {
                            (true, "Eligible (Verified by MintGo)".to_string(), None)
                        }
                    } else {
                        (false, format!("Contract Reverted: {last_revert}"), None)
                    }
                }
            }
        };

        Ok(ComprehensiveDropReport {
            target,
            minted_supply,
            max_supply,
            is_sold_out,
            drop_mechanism,
            stages,
            seadrop_config: seadrop_public,
            wallet_status: WalletEligibilityStatus {
                wallet_address: wallet_address.to_string(),
                is_eligible,
                remaining_mintable: remaining,
                minted_by_wallet,
                max_per_wallet,
                price_wei,
                price_eth,
                status_reason,
                simulated_gas_used: simulated_gas,
            },
        })
    }

    /// Formats the comprehensive drop report into a human-readable terminal dashboard.
    pub fn format_report(&self, report: &ComprehensiveDropReport) -> String {
        let max_sup_str = report.max_supply.map_or("Unlimited / Open Edition".to_string(), |m| format!("{m}"));
        let opensea_link = report.target.opensea_url.as_deref().unwrap_or("N/A");

        let mut out = String::new();
        out.push_str("\n═══════════════════════════════════════════════════════════════════════════════\n");
        out.push_str("              🎯 MINT ELIGIBILITY & OPENSEA DROP REPORT                      \n");
        out.push_str("═══════════════════════════════════════════════════════════════════════════════\n");
        out.push_str(&format!("  Collection:       {}\n", report.target.collection_name));
        out.push_str(&format!("  Contract:         {}\n", report.target.contract_address));
        out.push_str(&format!("  Network:          {} (Chain ID: {})\n", report.target.chain.to_uppercase(), report.target.chain_id));
        out.push_str(&format!("  OpenSea Link:     {}\n", opensea_link));
        out.push_str(&format!("  Drop Protocol:    {}\n\n", report.drop_mechanism));

        out.push_str("  📊 Supply Status:\n");
        out.push_str(&format!("     Minted Supply: {} / {}\n", report.minted_supply, max_sup_str));
        out.push_str(&format!("     Sold Out:      {}\n\n", if report.is_sold_out { "YES ❌" } else { "NO 🟢" }));

        out.push_str(&format!("  🎫 Wallet Status ({}):\n", report.wallet_status.wallet_address));
        out.push_str(&format!("     Minted So Far: {}\n", report.wallet_status.minted_by_wallet));
        out.push_str(&format!("     Max Per Wallet:{}\n", report.wallet_status.max_per_wallet));
        out.push_str(&format!("     Remaining:     {}\n", report.wallet_status.remaining_mintable));
        out.push_str(&format!("     Unit Price:    {:.6} ETH ({} Wei)\n\n", report.wallet_status.price_eth, report.wallet_status.price_wei));

        if !report.stages.is_empty() {
            out.push_str("  🗓️ Mint Stages (MintGo / OpenSea Radar):\n");
            for (idx, st) in report.stages.iter().enumerate() {
                let status_icon = if st.is_active_now { "🟢 ACTIVE" } else { "⚪ INACTIVE" };
                out.push_str(&format!("     {}. [{}] {} - {:.4} ETH (Max: {})\n", idx + 1, status_icon, st.label, st.price_eth, st.max_per_wallet.map_or("N/A".to_string(), |m| m.to_string())));
            }
            out.push('\n');
        }

        out.push_str("  🏁 Eligibility Verdict:\n");
        if report.wallet_status.is_eligible {
            let gas_info = report.wallet_status.simulated_gas_used.map_or(String::new(), |g| format!(" (Simulated Gas: {g})"));
            out.push_str(&format!("     ✅ ELIGIBLE TO MINT! (Can mint up to {} tokens){gas_info}\n", report.wallet_status.remaining_mintable));
        } else {
            out.push_str(&format!("     ⛔ NOT ELIGIBLE: {}\n", report.wallet_status.status_reason));
        }
        out.push_str("═══════════════════════════════════════════════════════════════════════════════\n");
        out
    }
}
