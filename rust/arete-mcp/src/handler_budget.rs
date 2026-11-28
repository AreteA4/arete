//! A cap on the request handlers running at once, cancelled requests'
//! included, and on the replies not yet written.
//!
//! rmcp runs each request's handler in a task of its own. A
//! `notifications/cancelled` only cancels the request's token, which the
//! handlers here do not watch, so the handler runs on to its end, and rmcp
//! then drops its reply unsent. The stdio transport forgets a cancelled
//! request at once, as no reply will answer it, so its place under the
//! transport's cap on requests awaiting a reply is free again while its
//! handler still runs. Nothing is written for such a handler either, so a
//! stdout the client does not read holds nothing back. A client that sent
//! requests to a slow tool and cancelled each one could so start handlers
//! without end, each with a task and the memory its call holds.
//!
//! A [`Budget`] holds [`MAX_RUNNING_HANDLERS`] places. The stdio transport
//! takes one as it admits a request ([`StdioTransport::with_budget`]) and
//! answers a request it finds none for with a `Server busy` error (code
//! -32000) itself, written before any further input is read, as it answers
//! one over its own cap: refusals so never pile up behind a stalled stdout.
//! The place, an [`Admission`], rides on the request to its handler, and is
//! given back only once the handler has finished, whether or not the
//! request was cancelled, and the transport is done with the request: its
//! reply written, or the request cancelled. [`HandlerBudget`] holds it for
//! the handler, and takes a place itself for a request that arrives
//! without one.
//!
//! Handlers are not stopped on cancellation: dropping one part way could
//! cut a tool off between a change and the record of it (a transaction
//! sent but its answer never built, a dev environment change half made),
//! and would bound only handlers that yield. Counting them until they end
//! bounds every handler, whatever it awaits, and leaves each tool behaving
//! as it does for a client that is still waiting.
//!
//! [`StdioTransport::with_budget`]: crate::stdio_framing::StdioTransport::with_budget

use std::borrow::Cow;
use std::future::Future;
use std::sync::Arc;

use rmcp::model::{ErrorCode, ErrorData, ProtocolVersion};
use rmcp::service::{MaybeSendFuture, NotificationContext, RequestContext, ServiceRole};
use rmcp::{RoleServer, Service};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Cap on requests whose handler is running or whose reply is not yet
/// written, cancelled requests' included until their handler finishes. The
/// same as the stdio transport's cap on requests awaiting a reply, which
/// counts only requests not cancelled.
pub const MAX_RUNNING_HANDLERS: usize = 1024;

/// The code of the error answering a request over the cap: the one the
/// stdio transport answers a request over its own budget with.
const SERVER_BUSY: ErrorCode = ErrorCode(-32000);

/// Why a request over the cap is refused, as reported to the client.
pub const OVER_BUDGET: &str = "too many requests are still being handled, cancelled ones included";

/// The places under [`MAX_RUNNING_HANDLERS`], shared by the transport and
/// [`HandlerBudget`].
#[derive(Clone)]
pub struct Budget(Arc<Semaphore>);

impl Budget {
    pub fn new() -> Self {
        Self(Arc::new(Semaphore::new(MAX_RUNNING_HANDLERS)))
    }

    /// A place, if one is free.
    pub fn admit(&self) -> Option<Admission> {
        let permit = Arc::clone(&self.0).try_acquire_owned().ok()?;
        Some(Admission {
            _permit: Arc::new(permit),
        })
    }
}

impl Default for Budget {
    fn default() -> Self {
        Self::new()
    }
}

/// A request's place under the [`Budget`], given back once every clone of
/// it is dropped.
#[derive(Clone)]
pub struct Admission {
    _permit: Arc<OwnedSemaphorePermit>,
}

/// `S`, with each request's handler holding its [`Admission`] until it
/// finishes; see the module docs.
pub struct HandlerBudget<S> {
    inner: S,
    budget: Budget,
}

impl<S> HandlerBudget<S> {
    pub fn new(inner: S, budget: Budget) -> Self {
        Self { inner, budget }
    }
}

impl<S: Service<RoleServer>> Service<RoleServer> for HandlerBudget<S> {
    fn handle_request(
        &self,
        request: <RoleServer as ServiceRole>::PeerReq,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<<RoleServer as ServiceRole>::Resp, ErrorData>> + MaybeSendFuture + '_
    {
        // Held by the handler's future, so the handler gives its place back
        // when it finishes. rmcp does not wait for handlers when the client
        // disconnects: one still running then keeps its place until it ends
        // on its own, as cancelling its token does not stop it.
        let admission = context
            .extensions
            .get::<Admission>()
            .cloned()
            .or_else(|| self.budget.admit());
        async move {
            let Some(_admission) = admission else {
                tracing::warn!(
                    running = MAX_RUNNING_HANDLERS,
                    "answering a request over the cap on running handlers with an error"
                );
                return Err(ErrorData::new(
                    SERVER_BUSY,
                    "Server busy",
                    Some(OVER_BUDGET.into()),
                ));
            };
            self.inner.handle_request(request, context).await
        }
    }

    fn handle_notification(
        &self,
        notification: <RoleServer as ServiceRole>::PeerNot,
        context: NotificationContext<RoleServer>,
    ) -> impl Future<Output = Result<(), ErrorData>> + MaybeSendFuture + '_ {
        self.inner.handle_notification(notification, context)
    }

    fn get_info(&self) -> <RoleServer as ServiceRole>::Info {
        self.inner.get_info()
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        self.inner.supported_protocol_versions()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use rmcp::model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ServerCapabilities, ServerConfig,
    };
    use rmcp::ServerHandler;
    use serde_json::{json, Value};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::sync::{mpsc, watch};

    use super::*;

    /// A server whose `slow` tool runs until `release` turns true, counted
    /// in `running` while it does, and whose other tools answer at once.
    struct Slow {
        running: Arc<AtomicUsize>,
        release: watch::Receiver<bool>,
    }

    /// One `slow` call in `running`, for as long as its handler runs.
    struct Running(Arc<AtomicUsize>);

    impl Running {
        fn new(running: &Arc<AtomicUsize>) -> Self {
            running.fetch_add(1, Ordering::SeqCst);
            Self(Arc::clone(running))
        }
    }

    impl Drop for Running {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    impl ServerHandler for Slow {
        fn get_info(&self) -> ServerConfig {
            ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
        }

        async fn call_tool(
            &self,
            request: CallToolRequestParams,
            _context: RequestContext<RoleServer>,
        ) -> Result<CallToolResponse, ErrorData> {
            if request.name == "slow" {
                let _running = Running::new(&self.running);
                let mut release = self.release.clone();
                let _ = release.wait_for(|released| *released).await;
            }
            Ok(CallToolResponse::Complete(CallToolResult::success(vec![])))
        }
    }

    fn line(message: Value) -> String {
        format!("{message}\n")
    }

    fn call(id: usize, tool: &str) -> String {
        line(json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": tool, "arguments": {} },
        }))
    }

    fn cancel(id: usize) -> String {
        line(json!({
            "jsonrpc": "2.0",
            "method": "notifications/cancelled",
            "params": { "requestId": id },
        }))
    }

    async fn next(replies: &mut mpsc::UnboundedReceiver<Value>) -> Value {
        tokio::time::timeout(Duration::from_secs(30), replies.recv())
            .await
            .expect("no reply within 30 s")
            .expect("the server closed stdout")
    }

    /// Wait until `ready` holds of `running`, or fail after 30 s.
    async fn until_running(running: &AtomicUsize, ready: impl Fn(usize) -> bool) {
        let waited = tokio::time::timeout(Duration::from_secs(30), async {
            while !ready(running.load(Ordering::SeqCst)) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        assert!(
            waited.is_ok(),
            "still {} slow handlers running after 30 s",
            running.load(Ordering::SeqCst)
        );
    }

    /// The reply to the request `id`. A cancelled call of the flood may be
    /// answered on the way: refused, when its handler had no place and
    /// rmcp took the refusal before the cancellation that followed it, but
    /// never with a result, as no call that ran is answered once cancelled.
    async fn reply_to(
        replies: &mut mpsc::UnboundedReceiver<Value>,
        id: usize,
        flood: usize,
    ) -> Value {
        loop {
            let reply = next(replies).await;
            if reply["id"] == json!(id) {
                return reply;
            }
            assert_flood_refusal(&reply, flood);
        }
    }

    fn assert_flood_refusal(reply: &Value, flood: usize) {
        let in_flood = reply["id"]
            .as_u64()
            .is_some_and(|id| (1..=flood as u64).contains(&id));
        assert!(in_flood, "an unexpected reply: {reply}");
        assert_eq!(reply["error"]["code"], json!(-32000), "{reply}");
    }

    #[tokio::test]
    async fn a_flood_of_cancelled_calls_to_a_slow_tool_keeps_handlers_and_tasks_bounded() {
        let flood = 3 * MAX_RUNNING_HANDLERS;
        let running = Arc::new(AtomicUsize::new(0));
        let (release, released) = watch::channel(false);
        let tasks_before = tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks();

        let (client, server_end) = tokio::io::duplex(64 * 1024);
        let (read, write) = tokio::io::split(server_end);
        let served = tokio::spawn(crate::serve_streams(
            Slow {
                running: Arc::clone(&running),
                release: released,
            },
            read,
            write,
        ));
        let (client_read, mut client_write) = tokio::io::split(client);
        let (replies_tx, mut replies) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut lines = BufReader::new(client_read).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = replies_tx.send(serde_json::from_str::<Value>(&line).unwrap());
            }
        });

        // The handshake, then every call to the slow tool cancelled right
        // after it is sent.
        let mut input = line(json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "cancel-flood", "version": "0" },
            },
        }));
        input += &line(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        // The last few calls are written apart from their cancellations, so
        // their refusals may well be answered before they are cancelled.
        let apart = flood - 8;
        for id in 1..=apart {
            input += &call(id, "slow");
            input += &cancel(id);
        }
        client_write.write_all(input.as_bytes()).await.unwrap();
        for id in apart + 1..=flood {
            client_write
                .write_all(call(id, "slow").as_bytes())
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(5)).await;
            client_write.write_all(cancel(id).as_bytes()).await.unwrap();
        }
        let initialized = reply_to(&mut replies, 0, flood).await;
        assert!(initialized["result"].is_object(), "{initialized}");

        // Every call that is going to start has once the cap is full, the
        // rest refused; give any left over the time to start, if they can.
        until_running(&running, |running| running >= MAX_RUNNING_HANDLERS).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        let started = running.load(Ordering::SeqCst);
        assert_eq!(
            started, MAX_RUNNING_HANDLERS,
            "{started} slow handlers running for {flood} cancelled calls"
        );
        let tasks = tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks();
        assert!(
            tasks <= tasks_before + MAX_RUNNING_HANDLERS + 16,
            "{tasks} tasks alive for {MAX_RUNNING_HANDLERS} running handlers"
        );

        // A call nobody cancels is refused while the cancelled calls still
        // running fill the cap.
        let probe = flood + 1;
        client_write
            .write_all(call(probe, "fast").as_bytes())
            .await
            .unwrap();
        let refused = reply_to(&mut replies, probe, flood).await;
        assert_eq!(refused["error"]["code"], json!(-32000), "{refused}");
        assert_eq!(
            refused["error"]["message"],
            json!("Server busy"),
            "{refused}"
        );

        // Once the slow calls finish, every place is free again, and their
        // replies are never written: a second batch of slow calls, none
        // cancelled, is all admitted at once, and each is answered with its
        // result once released.
        release.send(true).unwrap();
        until_running(&running, |running| running == 0).await;
        release.send(false).unwrap();
        let batch = flood + 2..flood + 2 + MAX_RUNNING_HANDLERS;
        let second: String = batch.clone().map(|id| call(id, "slow")).collect();
        client_write.write_all(second.as_bytes()).await.unwrap();
        until_running(&running, |running| running == MAX_RUNNING_HANDLERS).await;
        release.send(true).unwrap();
        let mut answered = std::collections::BTreeSet::new();
        while answered.len() < MAX_RUNNING_HANDLERS {
            let reply = next(&mut replies).await;
            let id = reply["id"].as_u64().map(|id| id as usize);
            match id.filter(|id| batch.contains(id)) {
                Some(id) => {
                    assert!(reply["result"].is_object(), "{reply}");
                    assert!(answered.insert(id), "answered twice: {reply}");
                }
                None => assert_flood_refusal(&reply, flood),
            }
        }
        until_running(&running, |running| running == 0).await;

        client_write.shutdown().await.unwrap();
        served.await.unwrap().unwrap();
        while let Some(reply) = replies.recv().await {
            assert_flood_refusal(&reply, flood);
        }
    }

    fn initialize() -> String {
        line(json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "cancel-flood", "version": "0" },
            },
        })) + &line(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
    }

    #[tokio::test]
    async fn refusals_over_the_cap_wait_on_a_stalled_stdout_rather_than_pile_up() {
        let running = Arc::new(AtomicUsize::new(0));
        let (release, released) = watch::channel(false);
        let tasks_before = tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks();

        // Small pipes, so a client that stops reading stalls stdout soon.
        let (client, server_end) = tokio::io::duplex(4 * 1024);
        let (read, write) = tokio::io::split(server_end);
        let served = tokio::spawn(crate::serve_streams(
            Slow {
                running: Arc::clone(&running),
                release: released,
            },
            read,
            write,
        ));
        let (client_read, mut client_write) = tokio::io::split(client);
        let mut lines = BufReader::new(client_read).lines();
        client_write
            .write_all(initialize().as_bytes())
            .await
            .unwrap();
        let initialized: Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(initialized["id"], json!(0), "{initialized}");

        // Cancelled slow calls fill the cap; nothing is written for them.
        let mut held = String::new();
        for id in 1..=MAX_RUNNING_HANDLERS {
            held += &call(id, "slow");
            held += &cancel(id);
        }
        client_write.write_all(held.as_bytes()).await.unwrap();
        until_running(&running, |running| running == MAX_RUNNING_HANDLERS).await;

        // Then, with stdout no longer read, calls that every one is refused.
        let refused = MAX_RUNNING_HANDLERS + 1..=5 * MAX_RUNNING_HANDLERS;
        let flood: String = refused.clone().map(|id| call(id, "fast")).collect();
        let writer = tokio::spawn(async move {
            client_write.write_all(flood.as_bytes()).await.unwrap();
            client_write
        });
        tokio::time::sleep(Duration::from_millis(300)).await;
        let tasks = tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks();
        assert!(
            tasks <= tasks_before + MAX_RUNNING_HANDLERS + 16,
            "{tasks} tasks alive behind a stalled stdout for {MAX_RUNNING_HANDLERS} running handlers"
        );
        assert!(
            !writer.is_finished(),
            "the server read all of the input while it could not write its refusals"
        );

        // Read again: each call is refused, in order, and the rest of the
        // input is taken.
        for id in refused {
            let reply: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(reply["id"], json!(id), "{reply}");
            assert_eq!(reply["error"]["code"], json!(-32000), "{reply}");
        }
        let mut client_write = writer.await.unwrap();
        release.send(true).unwrap();
        until_running(&running, |running| running == 0).await;
        client_write.shutdown().await.unwrap();
        served.await.unwrap().unwrap();
        assert!(lines.next_line().await.unwrap().is_none());
    }
}
