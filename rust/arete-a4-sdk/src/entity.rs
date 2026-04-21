/// Stack definition trait - defines the shape of a Arete deployment.
///
/// ```ignore
/// use arete_sdk::{Stack, Views};
///
/// pub struct OreStack;
///
/// impl Stack for OreStack {
///     type Views = OreRoundViews;
///
///     fn name() -> &'static str { "ore-round" }
///     fn url() -> &'static str { "wss://ore.stack.arete.run" }
/// }
///
/// // Usage
/// let a4 = Arete::<OreStack>::connect().await?;
/// let rounds = a4.views.latest().get().await;
/// ```
pub trait Stack: Sized + Send + Sync + 'static {
    type Views: crate::view::Views;

    fn name() -> &'static str;
    fn url() -> &'static str;
}
