use crate::tests::test_runner::TestRunner;
use anyhow::Context;
use rrelayer::ApiSdkError;
use rrelayer_core::transaction::api::{RelayTransactionRequest, TransactionSpeed};
use rrelayer_core::transaction::types::{TransactionData, TransactionStatus};
use tracing::info;

impl TestRunner {
    /// RRELAYER_PROVIDERS="raw" make run-test-debug TEST=transaction_replace
    pub async fn transaction_replace(&self) -> anyhow::Result<()> {
        let relayer = self.create_and_fund_relayer("tx-replace-relayer").await?;
        for pending in [true, false] {
            for repeat_external_id in [false, true] {
                if pending {
                    // A one-wei gas cap is below Anvil fees and holds the transaction pending.
                    relayer.update_max_gas_price(1).await?;
                }
                let tx_request = RelayTransactionRequest {
                    to: self.config.anvil_accounts[1],
                    value: alloy::primitives::utils::parse_ether("0.1")?.into(),
                    data: TransactionData::empty(),
                    speed: Some(TransactionSpeed::SLOW),
                    external_id: Some(format!("test-original-{pending}-{repeat_external_id}")),
                    blobs: None,
                };
                let send_result = relayer.transaction().send(&tx_request, None).await?;
                let expected_status =
                    if pending { TransactionStatus::PENDING } else { TransactionStatus::INMEMPOOL };
                let original = tokio::time::timeout(
                    std::time::Duration::from_secs(self.config.test_timeout_seconds),
                    async {
                        loop {
                            let transaction =
                                self.relayer_client.get_transaction(&send_result.id).await?;
                            if transaction.status == expected_status {
                                break anyhow::Ok(transaction);
                            }
                            anyhow::ensure!(
                                transaction.status == TransactionStatus::PENDING,
                                "Unexpected transaction status {}",
                                transaction.status
                            );
                        }
                    },
                )
                .await
                .context("Transaction did not reach replacement phase")??;
                let mut replacement_request = RelayTransactionRequest {
                    value: alloy::primitives::utils::parse_ether("0.2")?.into(),
                    external_id: Some("test-replacement".to_string()),
                    ..tx_request.clone()
                };
                let error = relayer
                    .transaction()
                    .replace(&send_result.id, &replacement_request, None)
                    .await
                    .err()
                    .context("Rebinding an external ID must fail in every phase")?;
                anyhow::ensure!(
                    matches!(error, ApiSdkError::HttpError(ref error)
                    if error.status().map(|status| status.as_u16()) == Some(409)),
                    "Expected HTTP 409, got {error}"
                );
                let unchanged = self.relayer_client.get_transaction(&send_result.id).await?;
                anyhow::ensure!(
                    serde_json::to_value(&unchanged)? == serde_json::to_value(&original)?,
                    "Conflicting replacement changed the original transaction"
                );
                anyhow::ensure!(
                    relayer.transaction().get_by_external_id("test-replacement").await?.is_none(),
                    "Conflicting replacement created a transaction"
                );

                replacement_request.external_id =
                    if repeat_external_id { tx_request.external_id.clone() } else { None };
                let replaced = relayer
                    .transaction()
                    .replace(&send_result.id, &replacement_request, None)
                    .await?;
                anyhow::ensure!(replaced.success, "Replacement failed");
                let replacement_id =
                    replaced.replace_transaction_id.context("Missing replacement ID")?;
                let original_after = self.relayer_client.get_transaction(&send_result.id).await?;
                let replacement = self.relayer_client.get_transaction(&replacement_id).await?;
                anyhow::ensure!(
                    original_after.external_id == tx_request.external_id,
                    "Original external ID changed"
                );
                anyhow::ensure!(replacement.nonce == original.nonce, "Replacement nonce changed");
                let mut expected_request = replacement_request.clone();
                if pending {
                    anyhow::ensure!(
                        replacement_id == send_result.id,
                        "Pending replacement must update the same row"
                    );
                    anyhow::ensure!(
                        original_after.cancelled_by_transaction_id.is_none(),
                        "Pending replacement must not create a successor"
                    );
                    expected_request.external_id = tx_request.external_id.clone();
                    relayer.remove_max_gas_price().await?;
                } else {
                    anyhow::ensure!(
                        replacement_id != send_result.id,
                        "In-mempool replacement must create a competitor"
                    );
                    anyhow::ensure!(
                        original_after.cancelled_by_transaction_id == Some(replacement_id),
                        "Original must link to its successor"
                    );
                    expected_request.external_id = Some(format!("replace_{}", send_result.id));
                }
                self.relayer_client.sent_transaction_compare(expected_request, replacement)?;
                let lookup = relayer
                    .transaction()
                    .get_by_external_id(tx_request.external_id.as_deref().unwrap())
                    .await?
                    .context("Original external ID no longer resolves")?;
                anyhow::ensure!(
                    lookup.id == send_result.id,
                    "Original lookup points to competitor"
                );
                self.anvil_manager.mine_and_wait().await?;
                self.wait_for_transaction_completion(&replacement_id).await?;
                if !pending {
                    let original_final =
                        self.wait_for_transaction_terminal(&send_result.id).await?;
                    anyhow::ensure!(
                        original_final.status == TransactionStatus::REPLACED,
                        "Original was not replaced"
                    );
                    anyhow::ensure!(
                        original_final.external_id == tx_request.external_id
                            && original_final.cancelled_by_transaction_id == Some(replacement_id),
                        "Original identity or successor link changed after mining"
                    );
                }
                info!("[SUCCESS] Replacement identity verified: pending={pending}, repeated_id={repeat_external_id}");
            }
        }
        Ok(())
    }
}
