//! A cap on the request handlers running at once, cancelled requests'
//! included, and on the replies not yet written or discarded.
//!
//! rmcp runs each request's handler in a task of its own. A
//! `notifications/cancelled` only cancels the request's token, which the
//! handlers here do not watch, so the handler runs on to its end, and its
//! reply is then dropped unsent. The stdio transport forgets a cancelled
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
//! reply written, or, for a cancelled request, its reply handed back by
//! rmcp and dropped. [`HandlerBudget`] holds it for the handler, and takes
//! a place itself for a request that arrives without one.
//!
//! The transport keeps a cancelled request's cancellation from rmcp, which
//! would otherwise drop the reply where the transport cannot see it, after
//! the handler gave its place back but while its task still waits to hand
//! the reply over: a burst of such replies would wait with no place held
//! while new requests took the places they freed. It cancels the request
//! through its [`Admission`] instead, which cancels the handler's token as
//! rmcp would have. A handler that finds its request cancelled drops its
//! result before handing it over, so what waits is a small error, never
//! the result of the call.
//!
//! A handler that panics is answered with an `Internal error` (code
//! -32603), so its request is answered and its place given back as for any
//! other reply; rmcp would otherwise drop the request without a reply and
//! the transport would keep its place until the session ends.
//!
//! Handlers are not stopped on cancellation: dropping one part way could
//! cut a tool off between a change and the record of it (a transaction
//! sent but its answer never built, a dev environment change half made),
//! and would bound only handlers that yield. Counting them until they end
//! bounds every handler, whatever it awaits, and leaves each tool behaving
//! as it does for a client that is still waiting.
//!
//! [`StdioTransport::with_budget`]: crate::stdio_framing::StdioTransport::with_budget

use std::any::Any;
use std::borrow::Cow;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex, PoisonError};

use futures_util::FutureExt;
use rmcp::model::{ErrorCode, ErrorData, ProtocolVersion};
use rmcp::service::{MaybeSendFuture, NotificationContext, RequestContext, ServiceRole};
use rmcp::{RoleServer, Service};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Cap on requests whose handler is running or whose reply is not yet
/// written, cancelled requests' included until their handler finishes and
/// their reply is dropped. The same as the stdio transport's cap on
/// requests awaiting a reply, which counts only requests not cancelled.
pub const MAX_RUNNING_HANDLERS: usize = 1024;

/// The code of the error answering a request over the cap: the one the
/// stdio transport answers a request over its own budget with.
const SERVER_BUSY: ErrorCode = ErrorCode(-32000);

/// Why a request over the cap is refused, as reported to the client.
pub const OVER_BUDGET: &str = "too many requests are still being handled, cancelled ones included";

/// Why a request whose handler panicked is answered with an error, as
/// reported to the client. What the panic said is logged, not sent.
const HANDLER_PANICKED: &str = "the request's handler panicked";

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
        Some(Admission(Arc::new(Place {
            _permit: permit,
            cancel: Mutex::new(Cancel::Open(None)),
        })))
    }

    /// How many places are free.
    #[cfg(test)]
    pub(crate) fn free(&self) -> usize {
        self.0.available_permits()
    }
}

impl Default for Budget {
    fn default() -> Self {
        Self::new()
    }
}

/// A request's place under the [`Budget`], given back once every clone of
/// it is dropped, and the way the transport cancels the request's handler
/// when it keeps the cancellation from rmcp; see the module docs.
#[derive(Clone)]
pub struct Admission(Arc<Place>);

struct Place {
    _permit: OwnedSemaphorePermit,
    cancel: Mutex<Cancel>,
}

/// Whether the request is cancelled, and what to run when it is.
enum Cancel {
    Open(Option<Box<dyn FnOnce() + Send>>),
    Cancelled,
}

impl Admission {
    /// Run `hook` once the request is cancelled: now, if it already is.
    pub(crate) fn on_cancel(&self, hook: impl FnOnce() + Send + 'static) {
        let mut cancel = self.0.cancel.lock().unwrap_or_else(PoisonError::into_inner);
        match &mut *cancel {
            Cancel::Open(waiting) => *waiting = Some(Box::new(hook)),
            Cancel::Cancelled => {
                drop(cancel);
                hook();
            }
        }
    }

    /// Cancel the request, running the hook given to `on_cancel`, if any.
    pub(crate) fn cancel(&self) {
        let hook = {
            let mut cancel = self.0.cancel.lock().unwrap_or_else(PoisonError::into_inner);
            match std::mem::replace(&mut *cancel, Cancel::Cancelled) {
                Cancel::Open(hook) => hook,
                Cancel::Cancelled => None,
            }
        };
        if let Some(hook) = hook {
            hook();
        }
    }
}

/// `S`, with each request's handler holding its [`Admission`] until it
/// finishes, and a panic in a handler answered with an error; see the
/// module docs.
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
        let token = context.ct.clone();
        if let Some(admission) = &admission {
            let token = token.clone();
            admission.on_cancel(move || token.cancel());
        }
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
            // Unwinding is caught at the handler, so the request is still
            // answered. Its future is dropped with the panic, as rmcp drops
            // it when it catches none.
            let handled = AssertUnwindSafe(self.inner.handle_request(request, context))
                .catch_unwind()
                .await;
            let result = handled.unwrap_or_else(|panic| {
                tracing::error!(
                    panic = panic_message(&*panic),
                    "answering a request whose handler panicked with an error"
                );
                Err(ErrorData::internal_error(
                    "Internal error",
                    Some(HANDLER_PANICKED.into()),
                ))
            });
            if token.is_cancelled() {
                // No reply is written for a cancelled request, nor once the
                // session is over. The result is dropped here, under the
                // place, rather than wait with rmcp to be dropped there.
                drop(result);
                return Err(ErrorData::internal_error("Request cancelled", None));
            }
            result
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

/// What a panic said, when it said it with a string.
fn panic_message(panic: &(dyn Any + Send)) -> &str {
    panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("a panic without a message")
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use rmcp::model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ServerCapabilities,
        ServerConfig,
    };
    use rmcp::ServerHandler;
    use serde_json::{json, Value};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::sync::{mpsc, watch};

    use super::*;

    /// A server whose `slow` tool runs until `release` turns true, counted
    /// in `running` while it does, and whose other tools answer at once,
    /// but for those of [`Tools`].
    struct Slow {
        running: Arc<AtomicUsize>,
        release: watch::Receiver<bool>,
        tools: Tools,
    }

    /// The rest of [`Slow`]'s tools: `hold`, which runs until `hold` turns
    /// true, counted in `held` while it does, and records in `peak_tasks`
    /// the most tasks alive as one starts; `panic`, which panics; and
    /// `big`, which answers with a MiB of text. `cancelled` counts the
    /// `slow` and `hold` calls that found their token cancelled as they
    /// finished.
    struct Tools {
        held: Arc<AtomicUsize>,
        hold: watch::Receiver<bool>,
        peak_tasks: Arc<AtomicUsize>,
        cancelled: Arc<AtomicUsize>,
    }

    impl Default for Tools {
        fn default() -> Self {
            Self {
                held: Arc::default(),
                hold: watch::channel(true).1,
                peak_tasks: Arc::default(),
                cancelled: Arc::default(),
            }
        }
    }

    impl Slow {
        fn new(running: &Arc<AtomicUsize>, release: watch::Receiver<bool>) -> Self {
            Self {
                running: Arc::clone(running),
                release,
                tools: Tools::default(),
            }
        }
    }

    fn alive_tasks() -> usize {
        tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks()
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
            context: RequestContext<RoleServer>,
        ) -> Result<CallToolResponse, ErrorData> {
            let tools = &self.tools;
            let gate = match &*request.name {
                "slow" => Some((&self.running, &self.release)),
                "hold" => {
                    tools.peak_tasks.fetch_max(alive_tasks(), Ordering::SeqCst);
                    Some((&tools.held, &tools.hold))
                }
                "panic" => panic!("a tool that panics"),
                "big" => {
                    let text = ContentBlock::text("x".repeat(1 << 20));
                    return Ok(CallToolResponse::Complete(CallToolResult::success(vec![
                        text,
                    ])));
                }
                _ => None,
            };
            if let Some((count, gate)) = gate {
                let _running = Running::new(count);
                let _ = gate.clone().wait_for(|open| *open).await;
                if context.ct.is_cancelled() {
                    tools.cancelled.fetch_add(1, Ordering::SeqCst);
                }
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

        let budget = Budget::new();
        let (client, server_end) = tokio::io::duplex(64 * 1024);
        let (read, write) = tokio::io::split(server_end);
        let served = tokio::spawn(crate::serve_under(
            Slow::new(&running, released),
            read,
            write,
            budget.clone(),
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

        // Once the slow calls finish and their replies are dropped, every
        // place is free again, and those replies are never written: a second
        // batch of slow calls, none cancelled, is all admitted at once, and
        // each is answered with its result once released.
        release.send(true).unwrap();
        until_running(&running, |running| running == 0).await;
        until_free(&budget).await;
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
            Slow::new(&running, released),
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

    /// Wait until every place under `budget` is free, or fail after 30 s.
    async fn until_free(budget: &Budget) {
        let waited = tokio::time::timeout(Duration::from_secs(30), async {
            while budget.free() != MAX_RUNNING_HANDLERS {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        assert!(
            waited.is_ok(),
            "{} places still taken after 30 s",
            MAX_RUNNING_HANDLERS - budget.free()
        );
    }

    /// A session of `server` under `budget`, past its handshake: the input
    /// to it, and every reply it writes.
    struct Session {
        served: tokio::task::JoinHandle<anyhow::Result<()>>,
        input: tokio::io::WriteHalf<tokio::io::DuplexStream>,
        replies: mpsc::UnboundedReceiver<Value>,
    }

    async fn start(server: Slow, budget: &Budget) -> Session {
        let (client, server_end) = tokio::io::duplex(64 * 1024);
        let (read, write) = tokio::io::split(server_end);
        let served = tokio::spawn(crate::serve_under(server, read, write, budget.clone()));
        let (client_read, mut input) = tokio::io::split(client);
        let (replies_tx, mut replies) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut lines = BufReader::new(client_read).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = replies_tx.send(serde_json::from_str::<Value>(&line).unwrap());
            }
        });
        input.write_all(initialize().as_bytes()).await.unwrap();
        let initialized = next(&mut replies).await;
        assert!(initialized["result"].is_object(), "{initialized}");
        Session {
            served,
            input,
            replies,
        }
    }

    #[tokio::test]
    async fn a_burst_of_cancelled_completions_under_churn_keeps_every_handler_task_under_a_place() {
        let budget = Budget::new();
        let running = Arc::new(AtomicUsize::new(0));
        let (release, released) = watch::channel(false);
        let (unhold, hold) = watch::channel(false);
        let mut server = Slow::new(&running, released);
        server.tools.hold = hold;
        let held = Arc::clone(&server.tools.held);
        let peak_tasks = Arc::clone(&server.tools.peak_tasks);
        let cancelled = Arc::clone(&server.tools.cancelled);
        let tasks_before = alive_tasks();
        let mut session = start(server, &budget).await;

        // Cancelled slow calls fill the cap.
        let fill: String = (1..=MAX_RUNNING_HANDLERS)
            .map(|id| call(id, "slow") + &cancel(id))
            .collect();
        session.input.write_all(fill.as_bytes()).await.unwrap();
        until_running(&running, |running| running == MAX_RUNNING_HANDLERS).await;
        assert_eq!(budget.free(), 0);

        // Every one finishes at once, its reply to be dropped, while more
        // cancelled calls arrive, to a tool that holds them. Each of those
        // starts only once a place is free, which is once a reply of the
        // burst has been dropped, not as soon as its handler ended: the
        // tasks still handing a reply over count under the cap.
        let flood = 3 * MAX_RUNNING_HANDLERS;
        let churn: String = (MAX_RUNNING_HANDLERS + 1..=flood)
            .map(|id| call(id, "hold") + &cancel(id))
            .collect();
        release.send(true).unwrap();
        session.input.write_all(churn.as_bytes()).await.unwrap();
        let probe = flood + 1;
        session
            .input
            .write_all(call(probe, "fast").as_bytes())
            .await
            .unwrap();
        reply_to(&mut session.replies, probe, flood).await;
        until_running(&running, |running| running == 0).await;
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Each slow handler still saw its request cancelled.
        assert_eq!(cancelled.load(Ordering::SeqCst), MAX_RUNNING_HANDLERS);
        let holding = held.load(Ordering::SeqCst);
        assert!(holding > 0, "no held call started after the burst");
        assert_eq!(
            budget.free(),
            MAX_RUNNING_HANDLERS - holding,
            "places taken beyond the {holding} held calls running"
        );
        let peak = peak_tasks.load(Ordering::SeqCst);
        assert!(
            peak <= tasks_before + MAX_RUNNING_HANDLERS + 32,
            "{peak} tasks alive as a held call started, {tasks_before} before the session"
        );

        // Released, the held calls give every place back, and no cancelled
        // call is ever answered with a result.
        unhold.send(true).unwrap();
        until_running(&held, |held| held == 0).await;
        until_free(&budget).await;
        assert_eq!(
            cancelled.load(Ordering::SeqCst),
            MAX_RUNNING_HANDLERS + holding
        );
        let last = probe + 1;
        session
            .input
            .write_all(call(last, "fast").as_bytes())
            .await
            .unwrap();
        let answered = reply_to(&mut session.replies, last, flood).await;
        assert!(answered["result"].is_object(), "{answered}");
        session.input.shutdown().await.unwrap();
        session.served.await.unwrap().unwrap();
        while let Some(reply) = session.replies.recv().await {
            assert_flood_refusal(&reply, flood);
        }
    }

    #[tokio::test]
    async fn a_handler_that_panics_is_answered_with_an_internal_error_and_gives_its_place_back() {
        let budget = Budget::new();
        let running = Arc::new(AtomicUsize::new(0));
        let (_release, released) = watch::channel(false);
        let mut session = start(Slow::new(&running, released), &budget).await;
        let calls = call(1, "panic") + &call(2, "fast") + &call(3, "panic");
        session.input.write_all(calls.as_bytes()).await.unwrap();
        let mut replies = std::collections::BTreeMap::new();
        for _ in 0..3 {
            let reply = next(&mut session.replies).await;
            replies.insert(reply["id"].as_u64().unwrap(), reply);
        }
        for id in [1, 3] {
            let reply = &replies[&id];
            assert_eq!(reply["error"]["code"], json!(-32603), "{reply}");
            assert_eq!(
                reply["error"]["message"],
                json!("Internal error"),
                "{reply}"
            );
            assert_eq!(reply["error"]["data"], json!(HANDLER_PANICKED), "{reply}");
        }
        assert!(replies[&2]["result"].is_object(), "{}", replies[&2]);
        until_free(&budget).await;

        // The session goes on.
        session
            .input
            .write_all(call(4, "fast").as_bytes())
            .await
            .unwrap();
        let reply = next(&mut session.replies).await;
        assert_eq!(reply["id"], json!(4), "{reply}");
        assert!(reply["result"].is_object(), "{reply}");
        session.input.shutdown().await.unwrap();
        session.served.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn the_end_of_input_ends_the_session_and_frees_every_place_while_stdout_goes_unread() {
        let budget = Budget::new();
        let running = Arc::new(AtomicUsize::new(0));
        let (_release, released) = watch::channel(false);
        let (client, server_end) = tokio::io::duplex(4 * 1024);
        let (read, write) = tokio::io::split(server_end);
        let served = tokio::spawn(crate::serve_under(
            Slow::new(&running, released),
            read,
            write,
            budget.clone(),
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

        // Replies far bigger than the pipe, which the client leaves unread
        // and open as it ends its input.
        let calls: String = (1..=4).map(|id| call(id, "big")).collect();
        client_write.write_all(calls.as_bytes()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(budget.free(), MAX_RUNNING_HANDLERS - 4);
        client_write.shutdown().await.unwrap();

        // rmcp waits 5 s for the replies, then closes the transport, which
        // gives them up rather than wait on the writer they hold.
        tokio::time::timeout(Duration::from_secs(30), served)
            .await
            .expect("the session outlived its input behind an unread stdout")
            .unwrap()
            .unwrap();
        until_free(&budget).await;
        drop(lines);
    }
}
