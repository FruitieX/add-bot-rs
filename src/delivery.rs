//! Durable queue notifications; successful sends are removed only after a
//! receipt is saved. Telegram offers no idempotency key: an ambiguous network
//! failure or crash between send and receipt can still produce a duplicate.
use crate::{
    state::NotificationReceipt,
    state_container::StateContainer,
    util::{self, ReplyContext},
};
use chrono::{DateTime, Duration, Utc};
use color_eyre::Result;
use teloxide::{types::MessageId, Bot, RequestError};

pub async fn deliver_pending(
    bot: &Bot,
    sc: &StateContainer,
    id: u64,
    force: bool,
) -> Result<Option<MessageId>> {
    deliver_with(sc, id, Utc::now(), force, |destination, text| async move {
        util::send_msg_once(bot, destination, &text)
            .await
            .map(|message| message.id)
    })
    .await
}

async fn deliver_with<F, Fut>(
    sc: &StateContainer,
    id: u64,
    now: DateTime<Utc>,
    force: bool,
    send: F,
) -> Result<Option<MessageId>>
where
    F: FnOnce(ReplyContext, String) -> Fut,
    Fut: std::future::Future<Output = Result<MessageId, RequestError>>,
{
    let _serial = sc.delivery_lock.lock().await;
    let state = sc.read().await;
    if let Some(receipt) = state
        .notification_receipts
        .iter()
        .find(|receipt| receipt.id == id)
    {
        return Ok(Some(receipt.message_id));
    }
    if state.telegram_retry_until.is_some_and(|until| until > now) {
        return Ok(None);
    }
    let Some(notification) = state
        .pending_notifications
        .iter()
        .find(|notification| notification.id == id)
        .cloned()
    else {
        return Ok(None);
    };
    if notification.blocked
        || ((!force || notification.attempts > 0) && notification.next_attempt > now)
    {
        return Ok(None);
    }
    // Persist a delay before sending, so a crash doesn't immediately resend a
    // potentially accepted request. Normal errors replace this with backoff.
    sc.transact(move |state| {
        if let Some(notification) = state
            .pending_notifications
            .iter_mut()
            .find(|notification| notification.id == id)
        {
            notification.attempts = notification.attempts.saturating_add(1);
            notification.next_attempt = now + Duration::seconds(60);
        }
        Ok(())
    })
    .await?;
    let text = if now - notification.created_at > Duration::minutes(5) {
        format!(
            "Delayed queue notification · queued {} min ago\n\n{}",
            (now - notification.created_at).num_minutes(),
            notification.text
        )
    } else {
        notification.text
    };
    match send(notification.destination, text).await {
        Ok(message_id) => {
            sc.transact(move |state| {
                state
                    .pending_notifications
                    .retain(|notification| notification.id != id);
                state
                    .notification_receipts
                    .push(NotificationReceipt { id, message_id });
                if state.notification_receipts.len() > 256 {
                    state.notification_receipts.remove(0);
                }
                Ok(())
            })
            .await?;
            Ok(Some(message_id))
        }
        Err(error) => {
            let delay = util::telegram_retry_delay(&error, notification.attempts.saturating_add(1));
            let rate_limited = matches!(error, RequestError::RetryAfter(_));
            let retry_at =
                delay.map(|delay| Utc::now() + Duration::seconds(delay.as_secs() as i64));
            let summary = util::telegram_error_summary(&error);
            log::warn!(
                "Queue notification {id} delivery failed: {summary}; retryable={}",
                delay.is_some()
            );
            sc.transact(move |state| {
                if rate_limited {
                    state.telegram_retry_until = retry_at;
                }
                if let Some(notification) = state
                    .pending_notifications
                    .iter_mut()
                    .find(|notification| notification.id == id)
                {
                    notification.blocked = delay.is_none();
                    notification.last_failure = Some(summary);
                    if let Some(retry_at) = retry_at {
                        notification.next_attempt = retry_at;
                    }
                }
                Ok(())
            })
            .await?;
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_container::test_store;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use teloxide::{
        types::{ChatId, Seconds, ThreadId},
        ApiError,
    };

    async fn queued() -> (StateContainer, crate::state_container::TestDirectory, u64) {
        let (sc, directory) = test_store().await;
        let id = sc
            .transact(|state| {
                Ok(state.enqueue(
                    ReplyContext {
                        chat_id: ChatId(1),
                        thread_id: Some(ThreadId(MessageId(7))),
                        reply_to: Some(MessageId(12)),
                    },
                    "Ready".into(),
                    Utc::now(),
                ))
            })
            .await
            .unwrap();
        (sc, directory, id)
    }

    #[tokio::test]
    async fn rate_limited_notification_survives_restart_and_preserves_topic() {
        let (sc, directory, id) = queued().await;
        deliver_with(&sc, id, Utc::now(), true, |_, _| async {
            Err(RequestError::RetryAfter(Seconds::from_seconds(30)))
        })
        .await
        .unwrap();
        let state = sc.read().await;
        let pending = &state.pending_notifications[0];
        assert!(!pending.blocked);
        assert_eq!(pending.attempts, 1);
        assert!(pending.next_attempt > Utc::now() + Duration::seconds(29));
        let restored =
            StateContainer::load(directory.0.join("state.json"), chrono_tz::UTC, Utc::now())
                .await
                .unwrap();
        let other = restored
            .transact(|state| {
                Ok(state.enqueue(
                    ReplyContext {
                        chat_id: ChatId(2),
                        thread_id: None,
                        reply_to: None,
                    },
                    "Another queue".into(),
                    Utc::now(),
                ))
            })
            .await
            .unwrap();
        assert_eq!(
            deliver_with(&restored, other, Utc::now(), true, |_, _| async {
                panic!("global Telegram rate limit must defer other queue notifications")
            })
            .await
            .unwrap(),
            None
        );
        let attempted = Arc::new(AtomicUsize::new(0));
        let counter = attempted.clone();
        assert_eq!(
            deliver_with(&restored, id, Utc::now(), true, move |_, _| async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(MessageId(1))
            })
            .await
            .unwrap(),
            None
        );
        assert_eq!(attempted.load(Ordering::SeqCst), 0);
        let receipt = deliver_with(
            &restored,
            id,
            pending.next_attempt,
            false,
            |destination, text| async move {
                assert_eq!(destination.thread_id, Some(ThreadId(MessageId(7))));
                assert_eq!(destination.reply_to, Some(MessageId(12)));
                assert_eq!(text, "Ready");
                Ok(MessageId(42))
            },
        )
        .await
        .unwrap();
        assert_eq!(receipt, Some(MessageId(42)));
        assert_eq!(restored.read().await.pending_notifications.len(), 1);
        assert_eq!(restored.read().await.pending_notifications[0].id, other);
        let again =
            StateContainer::load(directory.0.join("state.json"), chrono_tz::UTC, Utc::now())
                .await
                .unwrap();
        assert_eq!(
            again.read().await.notification_receipts[0].message_id,
            MessageId(42)
        );
    }

    #[tokio::test]
    async fn concurrent_delivery_sends_once_and_reuses_receipt() {
        let (sc, _directory, id) = queued().await;
        let count = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for _ in 0..2 {
            let sc = sc.clone();
            let count = count.clone();
            tasks.push(tokio::spawn(async move {
                deliver_with(&sc, id, Utc::now(), true, move |_, _| async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    Ok(MessageId(23))
                })
                .await
                .unwrap()
            }));
        }
        for task in tasks {
            assert_eq!(task.await.unwrap(), Some(MessageId(23)));
        }
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert!(sc.read().await.pending_notifications.is_empty());
    }

    #[tokio::test]
    async fn permanent_failure_keeps_a_blocked_notification_without_retrying() {
        let (sc, _directory, id) = queued().await;
        deliver_with(&sc, id, Utc::now(), true, |_, _| async {
            Err(RequestError::Api(ApiError::BotBlocked))
        })
        .await
        .unwrap();
        let state = sc.read().await;
        assert!(state.pending_notifications[0].blocked);
        assert_eq!(state.pending_notifications.len(), 1);
        let result = deliver_with(
            &sc,
            id,
            Utc::now() + Duration::hours(1),
            true,
            |_, _| async { panic!("permanent errors must not be retried") },
        )
        .await
        .unwrap();
        assert_eq!(result, None);
    }
}
