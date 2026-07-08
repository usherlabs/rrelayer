use std::{collections::BTreeSet, error::Error, fmt};

use tokio::sync::Mutex;

use crate::transaction::types::TransactionNonce;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonceReservationError {
    reconciled_nonce: TransactionNonce,
    highest_active_reservation: TransactionNonce,
}

impl NonceReservationError {
    fn new(reconciled_nonce: TransactionNonce, highest_active_reservation: TransactionNonce) -> Self {
        Self { reconciled_nonce, highest_active_reservation }
    }
}

impl fmt::Display for NonceReservationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Cannot set reconciled nonce {} below highest active reservation {}",
            self.reconciled_nonce.into_inner(),
            self.highest_active_reservation.into_inner()
        )
    }
}

impl Error for NonceReservationError {}

#[derive(Debug)]
struct NonceState {
    nonce: TransactionNonce,
    active_reservations: BTreeSet<u64>,
}

pub struct NonceManager {
    nonce: Mutex<NonceState>,
}

impl NonceManager {
    pub fn new(current_nonce: TransactionNonce) -> Self {
        NonceManager {
            nonce: Mutex::new(NonceState {
                nonce: current_nonce,
                active_reservations: BTreeSet::new(),
            }),
        }
    }

    pub async fn get_and_increment(&self) -> TransactionNonce {
        let mut nonce_guard = self.nonce.lock().await;
        let current_nonce = nonce_guard.nonce;
        nonce_guard.active_reservations.insert(current_nonce.into_inner());
        nonce_guard.nonce = current_nonce + 1;
        current_nonce
    }

    pub async fn sync_with_onchain_nonce(&self, onchain_nonce: TransactionNonce) {
        let mut nonce_guard = self.nonce.lock().await;
        if onchain_nonce.into_inner() > nonce_guard.nonce.into_inner() {
            nonce_guard.nonce = onchain_nonce;
        }
    }

    pub async fn set_reconciled_nonce(
        &self,
        reconciled_nonce: TransactionNonce,
    ) -> Result<(), NonceReservationError> {
        let mut nonce_guard = self.nonce.lock().await;
        if let Some(highest_active_reservation) =
            nonce_guard.active_reservations.iter().next_back().copied()
        {
            if reconciled_nonce.into_inner() <= highest_active_reservation {
                return Err(NonceReservationError::new(
                    reconciled_nonce,
                    TransactionNonce::new(highest_active_reservation),
                ));
            }
        }
        nonce_guard.nonce = reconciled_nonce;
        Ok(())
    }

    pub async fn get_current_nonce(&self) -> TransactionNonce {
        let nonce_guard = self.nonce.lock().await;
        nonce_guard.nonce
    }

    pub async fn release_unbroadcast_nonce(&self, nonce: TransactionNonce) {
        let mut nonce_guard = self.nonce.lock().await;
        nonce_guard.active_reservations.remove(&nonce.into_inner());
        if nonce_guard.nonce.into_inner() == nonce.into_inner() + 1 {
            nonce_guard.nonce = nonce;
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

        nonce_manager.set_reconciled_nonce(TransactionNonce::new(7)).await.unwrap();

        assert_eq!(nonce_manager.get_current_nonce().await, TransactionNonce::new(7));
        assert_eq!(nonce_manager.get_and_increment().await, TransactionNonce::new(7));
    }

    #[tokio::test]
    async fn set_reconciled_nonce_rejects_lowering_below_active_reservations() {
        let nonce_manager = NonceManager::new(TransactionNonce::new(7));
        let reserved = nonce_manager.get_and_increment().await;

        let err = nonce_manager.set_reconciled_nonce(TransactionNonce::new(7)).await.unwrap_err();

        assert_eq!(
            err.to_string(),
            "Cannot set reconciled nonce 7 below highest active reservation 7"
        );
        assert_eq!(reserved, TransactionNonce::new(7));
        assert_eq!(nonce_manager.get_current_nonce().await, TransactionNonce::new(8));
    }

    #[tokio::test]
    async fn set_reconciled_nonce_can_lower_after_releasing_active_reservation() {
        let nonce_manager = NonceManager::new(TransactionNonce::new(7));
        let reserved = nonce_manager.get_and_increment().await;
        nonce_manager.release_unbroadcast_nonce(reserved).await;

        nonce_manager.set_reconciled_nonce(TransactionNonce::new(7)).await.unwrap();

        assert_eq!(nonce_manager.get_current_nonce().await, TransactionNonce::new(7));
    }
}
