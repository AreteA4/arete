//! A cap on the request handlers running at once, cancelled requests'
//! included.
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
//! [`HandlerBudget`] counts every request from the moment its handler
//! starts until the handler finishes, whether or not the request was
//! cancelled, and answers a request past [`MAX_RUNNING_HANDLERS`] with a
//! `Server busy` error (code -32000) without running it, as the transport
//! answers one past its own cap. Its error reaches the client unless that
//! request is cancelled too, in which case rmcp drops it like any other
//! reply to a cancelled request.
//!
//! Handlers are not stopped on cancellation: dropping one part way could
//! cut a tool off between a change and the record of it (a transaction
//! sent but its answer never built, a dev environment change half made),
//! and would bound only handlers that yield. Counting them until they end
//! bounds every handler, whatever it awaits, and leaves each tool behaving
//! as it does for a client that is still waiting.

use std::borrow::Cow;
use std::future::Future;
use std::sync::Arc;

use rmcp::model::{ErrorCode, ErrorData, ProtocolVersion};
use rmcp::service::{MaybeSendFuture, NotificationContext, RequestContext, ServiceRole};
use rmcp::{RoleServer, Service};
use tokio::sync::Semaphore;

/// Cap on request handlers running at once, cancelled requests' included
/// until their handler finishes. The same as the stdio transport's cap on
/// requests awaiting a reply, which counts only requests not cancelled.
pub const MAX_RUNNING_HANDLERS: usize = 1024;

/// The code of the error answering a request over the cap: the one the
/// stdio transport answers a request over its own budget with.
const SERVER_BUSY: ErrorCode = ErrorCode(-32000);

/// `S`, running at most [`MAX_RUNNING_HANDLERS`] request handlers at once;
/// see the module docs.
pub struct HandlerBudget<S> {
    inner: S,
    running: Arc<Semaphore>,
}

impl<S> HandlerBudget<S> {
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            running: Arc::new(Semaphore::new(MAX_RUNNING_HANDLERS)),
        }
    }
}

impl<S: Service<RoleServer>> Service<RoleServer> for HandlerBudget<S> {
    fn handle_request(
        &self,
        request: <RoleServer as ServiceRole>::PeerReq,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<<RoleServer as ServiceRole>::Resp, ErrorData>> + MaybeSendFuture + '_
    {
        // Held by the handler's future, so its place is given back when the
        // handler finishes, or when rmcp drops it at shutdown.
        let permit = Arc::clone(&self.running).try_acquire_owned();
        async move {
            let Ok(_running) = permit else {
                tracing::warn!(
                    running = MAX_RUNNING_HANDLERS,
                    "answering a request over the cap on running handlers with an error"
                );
                return Err(ErrorData::new(
                    SERVER_BUSY,
                    "Server busy",
                    Some(
                        "too many requests are still being handled, cancelled ones included".into(),
                    ),
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

    /// Wait until `running` reads `want`, or fail after 30 s.
    async fn until_running(running: &AtomicUsize, want: usize) {
        let waited = tokio::time::timeout(Duration::from_secs(30), async {
            while running.load(Ordering::SeqCst) != want {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        assert!(
            waited.is_ok(),
            "{} slow handlers running, not {want}",
            running.load(Ordering::SeqCst)
        );
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
        // after it is sent, then a ping.
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
        for id in 1..=flood {
            input += &call(id, "slow");
            input += &cancel(id);
        }
        let ping = flood + 1;
        input += &line(json!({ "jsonrpc": "2.0", "id": ping, "method": "ping" }));
        let writer = tokio::spawn(async move {
            client_write.write_all(input.as_bytes()).await.unwrap();
            client_write
        });

        let initialized = next(&mut replies).await;
        assert_eq!(initialized["id"], json!(0), "{initialized}");
        assert!(initialized["result"].is_object(), "{initialized}");
        // No cancelled call is answered, so the next reply is the ping's.
        let pinged = next(&mut replies).await;
        // Let every call that was sent start, if it is going to.
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
        // The ping is refused, as the cancelled calls still running fill
        // the cap.
        assert_eq!(pinged["id"], json!(ping), "{pinged}");
        assert_eq!(pinged["error"]["code"], json!(-32000), "{pinged}");
        assert_eq!(pinged["error"]["message"], json!("Server busy"), "{pinged}");

        // Once the slow calls finish, their places are free again, and
        // their replies are never written.
        release.send(true).unwrap();
        until_running(&running, 0).await;
        let mut client_write = writer.await.unwrap();
        let fast = flood + 2;
        client_write
            .write_all(call(fast, "fast").as_bytes())
            .await
            .unwrap();
        let answered = next(&mut replies).await;
        assert_eq!(answered["id"], json!(fast), "{answered}");
        assert!(answered["result"].is_object(), "{answered}");

        client_write.shutdown().await.unwrap();
        served.await.unwrap().unwrap();
        assert!(replies.recv().await.is_none());
    }
}
