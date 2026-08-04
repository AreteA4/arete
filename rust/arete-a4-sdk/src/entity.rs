/// Stack definition trait - defines the shape of a Arete deployment.
///
/// ```ignore
/// use arete_sdk::{Stack, Views};
///
/// pub struct OreStack;
///
/// impl Stack for OreStack {
///     type Views = OreStackViews;
///     type Programs = OreStackPrograms; // `()` for stacks without programs
///
///     fn name() -> &'static str { "ore-stream" }
///     fn url() -> &'static str { "wss://ore.stack.arete.run" }
/// }
///
/// // Usage
/// let a4 = Arete::<OreStack>::connect().await?;
/// let rounds = a4.views.ore_round.latest().get().await;
/// let ix = a4.programs.ore.deploy(params)?;
/// ```
pub trait Stack: Sized + Send + Sync + 'static {
    type Views: crate::view::Views;

    /// Generated program SDK accessors bundled with this stack.
    /// Stacks without programs use `()`.
    type Programs: crate::program::Programs;

    fn name() -> &'static str;
    fn url() -> &'static str;

    /// The stack's generated HTTP endpoint (`endpoints.http`).
    ///
    /// Defaults to `""`; when empty, the client derives the HTTP base from
    /// the effective WebSocket URL via
    /// [`crate::chain::derive_http_endpoint`]. Divergence from TypeScript
    /// (which requires an explicit `httpUrl`/`endpoints.http`): deriving the
    /// endpoint from the WebSocket URL is the Rust default because both
    /// surfaces are served from the same host on every known deployment.
    fn http_url() -> &'static str {
        ""
    }
}
