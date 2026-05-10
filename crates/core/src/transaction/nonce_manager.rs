use tokio::sync::Mutex;

use crate::transaction::types::TransactionNonce;

pub struct NonceManager {
    nonce: Mutex<TransactionNonce>,
}

impl NonceManager {
    pub fn new(current_nonce: TransactionNonce) -> Self {
        NonceManager { nonce: Mutex::new(current_nonce) }
    }

    pub async fn get_and_increment(&self) -> TransactionNonce {
        let mut nonce_guard = self.nonce.lock().await;
        let current_nonce = *nonce_guard;
        *nonce_guard = current_nonce + 1;
        current_nonce
    }

    pub async fn sync_with_onchain_nonce(&self, onchain_nonce: TransactionNonce) {
        let mut nonce_guard = self.nonce.lock().await;
        if onchain_nonce.into_inner() > nonce_guard.into_inner() {
            *nonce_guard = onchain_nonce;
        }
    }

    pub async fn set_reconciled_nonce(&self, reconciled_nonce: TransactionNonce) {
        let mut nonce_guard = self.nonce.lock().await;
        *nonce_guard = reconciled_nonce;
    }

    pub async fn get_current_nonce(&self) -> TransactionNonce {
        let nonce_guard = self.nonce.lock().await;
        *nonce_guard
    }

    pub async fn release_unbroadcast_nonce(&self, nonce: TransactionNonce) {
        let mut nonce_guard = self.nonce.lock().await;
        if nonce_guard.into_inner() == nonce.into_inner() + 1 {
            *nonce_guard = nonce;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn add_transaction_releases_unbroadcast_nonce_after_estimate_failure() {
        let nonce_manager = NonceManager::new(TransactionNonce::new(7));

        let reserved_nonce = nonce_manager.get_and_increment().await;
        nonce_manager.release_unbroadcast_nonce(reserved_nonce).await;

        assert_eq!(nonce_manager.get_current_nonce().await, TransactionNonce::new(7));
        assert_eq!(nonce_manager.get_and_increment().await, TransactionNonce::new(7));
    }

    #[tokio::test]
    async fn release_unbroadcast_nonce_does_not_rewind_later_reservations() {
        let nonce_manager = NonceManager::new(TransactionNonce::new(7));

        let stale_reserved_nonce = nonce_manager.get_and_increment().await;
        let _later_reserved_nonce = nonce_manager.get_and_increment().await;
        nonce_manager.release_unbroadcast_nonce(stale_reserved_nonce).await;

        assert_eq!(nonce_manager.get_current_nonce().await, TransactionNonce::new(9));
    }

    #[tokio::test]
    async fn set_reconciled_nonce_can_lower_after_terminalized_future_head() {
        let nonce_manager = NonceManager::new(TransactionNonce::new(54));

        nonce_manager.set_reconciled_nonce(TransactionNonce::new(7)).await;

        assert_eq!(nonce_manager.get_current_nonce().await, TransactionNonce::new(7));
        assert_eq!(nonce_manager.get_and_increment().await, TransactionNonce::new(7));
    }
}
