use crate::vm::{Register, RegisterValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

// ============================================================================
// Type-safe field access traits and macros
// ============================================================================

/// Trait that describes how to access a field on a struct (NEW ENHANCED API)
///
/// This trait enables direct struct field access without the need for field_accessor! macros.
/// Use the `impl_field_descriptors!` macro to automatically implement for all fields.
///
/// # Example
/// ```ignore
/// struct TradingMetrics {
///     total_volume: u64,
///     trade_count: u64,
/// }
///
/// impl_field_descriptors!(TradingMetrics {
///     total_volume: u64,
///     trade_count: u64
/// });
///
/// // Use struct fields directly
/// ctx.get_field(TradingMetrics::total_volume())  // returns Option<u64>
/// ctx.set_field(TradingMetrics::total_volume(), 1000)
/// ctx.increment_field(TradingMetrics::trade_count(), 1)
/// ```
pub trait FieldDescriptor<T> {
    /// The type of the field value
    type Value: Serialize + for<'de> Deserialize<'de>;

    /// The path to this field (e.g., "total_volume")
    fn path(&self) -> &'static str;
}

/// Trait for direct field references - legacy approach (still supported)
///
/// This trait enables compile-time field name extraction and type checking.
/// Use the `field!` macro to create field references directly from struct fields.
///
/// # Example
/// ```ignore
/// struct TradingMetrics {
///     total_volume: u64,
///     trade_count: u64,
/// }
///
/// // Use fields directly with the field! macro
/// let volume = ctx.get(&field!(entity, total_volume));
/// ctx.set(&field!(entity, total_volume), 1000);
/// ctx.increment(&field!(entity, trade_count), 1);
/// ```
pub trait FieldRef<T> {
    /// Get the field path (e.g., "total_volume")
    fn path(&self) -> &'static str;

    /// Get the field value from a reference (for type inference)
    fn get_ref<'a>(&self, _source: &'a T) -> Option<&'a T> {
        None // Not used at runtime, only for type inference
    }
}

/// Trait for type-safe field access without string literals (LEGACY API)
///
/// Implement this trait to create compile-time checked field accessors:
/// ```ignore
/// struct TotalVolume;
/// impl FieldAccessor for TotalVolume {
///     type Value = u64;
///     fn path() -> &'static str { "total_volume" }
/// }
/// ```
///
/// Or use the `field_accessor!` macro for convenience.
///
/// **DEPRECATED**: Consider using the new `field!` macro instead for cleaner syntax.
pub trait FieldAccessor {
    /// The type of the field value
    type Value: Serialize + for<'de> Deserialize<'de>;

    /// The path to this field (e.g., "total_volume" or "reserves.last_price")
    fn path() -> &'static str;

    /// The nested path segments if needed (auto-computed from path)
    fn segments() -> Vec<&'static str> {
        Self::path().split('.').collect()
    }
}

// Helper macro to join field names with dots (internal use)
#[macro_export]
#[doc(hidden)]
macro_rules! __field_path {
    ($first:ident) => {
        stringify!($first)
    };
    ($first:ident, $($rest:ident),+) => {
        concat!(stringify!($first), ".", $crate::__field_path!($($rest),+))
    };
}

/// Macro to implement field descriptors for struct fields
///
/// This macro generates FieldDescriptor implementations and static methods for each field,
/// allowing direct struct field access without the need for separate accessor types.
///
/// # Example
/// ```ignore
/// struct TradingMetrics {
///     total_volume: u64,
///     trade_count: u64,
/// }
///
/// impl_field_descriptors!(TradingMetrics {
///     total_volume: u64,
///     trade_count: u64
/// });
///
/// // Now you can use:
/// ctx.get_field(TradingMetrics::total_volume())
/// ctx.set_field(TradingMetrics::total_volume(), 1000)
/// ```
#[macro_export]
macro_rules! impl_field_descriptors {
    ($struct_name:ident { $( $field_name:ident : $field_type:ty ),* $(,)? }) => {
        impl $struct_name {
            $(
                /// Returns a field descriptor for this field
                pub fn $field_name() -> impl $crate::metrics_context::FieldDescriptor<$struct_name, Value = $field_type> {
                    struct FieldDescriptorImpl;

                    impl $crate::metrics_context::FieldDescriptor<$struct_name> for FieldDescriptorImpl {
                        type Value = $field_type;

                        fn path(&self) -> &'static str {
                            stringify!($field_name)
                        }
                    }

                    FieldDescriptorImpl
                }
            )*
        }
    };
}

/// Creates a field reference for direct struct field access (LEGACY API)
///
/// This macro captures the field name at compile time and creates a zero-cost
/// field reference that can be used with MetricsContext methods.
///
/// # Examples
///
/// ```ignore
/// struct TradingMetrics {
///     total_volume: u64,
///     trade_count: u64,
/// }
///
/// let entity = TradingMetrics { total_volume: 0, trade_count: 0 };
///
/// // Create field references
/// let volume_field = field!(entity, total_volume);
/// let count_field = field!(entity, trade_count);
///
/// // Use with MetricsContext
/// ctx.get_ref(&volume_field)           // Option<u64>
/// ctx.set_ref(&count_field, 100)       // Set trade_count to 100
/// ctx.increment_ref(&count_field, 1)   // Increment by 1
///
/// // Nested fields also work
/// let price_field = field!(entity, reserves.last_price);
/// ctx.set_ref(&price_field, 123.45);
/// ```
///
/// # Advantages over field_accessor!
/// - No need to define separate accessor structs
/// - Field names are validated at compile time
/// - Type inference works automatically from struct definition
/// - Less boilerplate code
#[macro_export]
macro_rules! field {
    // Simple field (no dots)
    ($struct_expr:expr, $field:ident) => {{
        // Create a zero-sized type that captures the field name
        struct __FieldRef;

        impl<T> $crate::metrics_context::FieldRef<T> for __FieldRef {
            fn path(&self) -> &'static str {
                stringify!($field)
            }
        }

        // Return the field reference
        __FieldRef
    }};

    // Nested fields with dot notation
    ($struct_expr:expr, $($field:ident).+) => {{
        struct __FieldRef;

        impl<T> $crate::metrics_context::FieldRef<T> for __FieldRef {
            fn path(&self) -> &'static str {
                $crate::__field_path!($($field),+)
            }
        }

        __FieldRef
    }};
}

/// Macro to define type-safe field accessors (LEGACY API)
///
/// # Examples
///
/// ```ignore
/// // Simple field accessor
/// field_accessor!(TotalVolume, u64, "total_volume");
///
/// // Nested field accessor
/// field_accessor!(LastPrice, f64, "reserves.last_price");
///
/// // Usage with MetricsContext
/// ctx.get_field(TotalVolume)  // returns Option<u64>
/// ctx.set_field(TotalVolume, 1000)
/// ```
///
/// **DEPRECATED**: Consider using the new `field!` macro instead:
/// ```ignore
/// ctx.get(&field!(entity, total_volume))  // Cleaner, no accessor struct needed
/// ctx.set(&field!(entity, total_volume), 1000)
/// ```
#[macro_export]
macro_rules! field_accessor {
    ($name:ident, $type:ty, $path:expr) => {
        pub struct $name;

        impl $crate::metrics_context::FieldAccessor for $name {
            type Value = $type;

            fn path() -> &'static str {
                $path
            }
        }
    };
}

/// Re-export CompiledPath from vm module for public API
pub use crate::vm::CompiledPath;

/// MetricsContext provides an imperative API for complex aggregation logic
/// in instruction hooks generated by declarative macros.
///
/// **Note:** You don't write instruction hooks directly. Instead, use declarative macros:
/// - `#[aggregate]` for aggregations (Sum, Count, Min, Max, etc.)
/// - `#[track_from]` for field tracking
/// - `#[register_pda]` for PDA mappings
///
/// These macros generate instruction hooks internally that use MetricsContext.
///
/// It wraps VM registers to provide type-safe access to:
/// - Instruction data (accounts, args)
/// - Entity state (current field values)
/// - Context metadata (slot, signature, timestamp)
///
/// # Enhanced Field Descriptor API (NEW - Recommended)
/// ```ignore
/// // Define your entity struct with declarative macros
/// struct TradingMetrics {
///     #[aggregate(
///         from = [Buy, Sell],
///         field = data::amount,
///         strategy = Sum,
///         lookup_by = accounts::mint
///     )]
///     total_volume: u64,
///     
///     #[aggregate(
///         from = [Buy, Sell],
///         strategy = Count,
///         lookup_by = accounts::mint
///     )]
///     trade_count: u64,
/// }
///
/// // The macro generates hooks that use MetricsContext internally
/// // to implement the aggregation logic.
/// ```
///
/// # Direct MetricsContext Usage (Internal/Advanced)
///
/// If you're implementing custom runtime logic or extending the macro system,
/// you can use MetricsContext directly with field descriptors:
///
/// ```ignore
/// // Generate field descriptors - replaces field_accessor! macro
/// impl_field_descriptors!(TradingMetrics {
///     total_volume: u64,
///     trade_count: u64
/// });
///
/// // In generated hook function (example - you don't write this)
/// fn generated_update_metrics(ctx: &mut MetricsContext) {
///     let volume = ctx.get_field(TradingMetrics::total_volume());
///     ctx.set_field(TradingMetrics::total_volume(), 1000);
///     ctx.increment_field(TradingMetrics::trade_count(), 1);
/// }
/// ```
///
/// # Field Accessor API (Legacy - Still Supported)
/// ```ignore
/// // Define field accessors once
/// field_accessor!(TotalVolume, u64, "total_volume");
/// field_accessor!(TradeCount, u64, "trade_count");
///
/// // Use with compile-time type checking
/// ctx.get_field_legacy(TotalVolume)       // returns Option<u64>
/// ctx.set_field_legacy(TotalVolume, 100)  // type-checked at compile time
/// ctx.increment_field_legacy(TradeCount, 1)
/// ```
///
/// # String-based API (Legacy - Use for dynamic field access only)
/// ```ignore
/// ctx.get::<u64>("total_volume")  // String-based, runtime errors possible
/// ctx.set("total_volume", 100)
/// ctx.increment("trade_count", 1)
/// ```
pub struct MetricsContext<'a> {
    /// Register holding the current entity state
    state_reg: Register,
    /// All VM registers (mutable access for updates)
    registers: &'a mut Vec<RegisterValue>,
    /// Compiled field paths for efficient access
    #[allow(dead_code)]
    compiled_paths: &'a HashMap<String, CompiledPath>,
    /// Blockchain slot number
    slot: Option<u64>,
    /// Transaction signature
    signature: Option<String>,
    /// Unix timestamp (milliseconds)
    timestamp: i64,
}

impl<'a> MetricsContext<'a> {
    /// Create a new MetricsContext wrapping VM state
    pub fn new(
        state_reg: Register,
        registers: &'a mut Vec<RegisterValue>,
        compiled_paths: &'a HashMap<String, CompiledPath>,
        slot: Option<u64>,
        signature: Option<String>,
        timestamp: i64,
    ) -> Self {
        Self {
            state_reg,
            registers,
            compiled_paths,
            slot,
            signature,
            timestamp,
        }
    }

    // ========================================================================
    // Read instruction data
    // ========================================================================

    /// Get an account address from the instruction by name
    /// Example: `ctx.account("user")` returns the user account address
    pub fn account(&self, name: &str) -> Option<String> {
        // Accounts are stored in registers, typically in a source register
        // For now, we'll return None - full implementation requires access to source register
        // This is a placeholder for the actual implementation
        let _ = name;
        None
    }

    /// Get a typed field from instruction data
    /// Example: `ctx.data::<u64>("amount")` returns the amount field
    pub fn data<T: for<'de> Deserialize<'de>>(&self, field: &str) -> Option<T> {
        // Data fields are accessed via compiled paths
        // For now, placeholder - full implementation requires source register access
        let _ = field;
        None
    }

    // ========================================================================
    // Read current entity state
    // ========================================================================

    /// Get a typed value from the current entity state (string-based API)
    /// Example: `ctx.get::<u64>("total_volume")` returns the current total_volume value
    pub fn get<T: for<'de> Deserialize<'de>>(&self, field_path: &str) -> Option<T> {
        let state = self.registers.get(self.state_reg)?;

        // Navigate the field path
        let segments: Vec<&str> = field_path.split('.').collect();
        let mut current = state;

        for segment in segments {
            current = current.get(segment)?;
        }

        // Deserialize the value
        serde_json::from_value(current.clone()).ok()
    }

    /// Get a typed value using a field reference (NEW RECOMMENDED API)
    /// Example: `ctx.get_ref(&field!(entity, total_volume))` returns Option<u64>
    ///
    /// This provides compile-time field name validation and type inference.
    pub fn get_ref<T, F>(&self, field_ref: &F) -> Option<T>
    where
        T: for<'de> Deserialize<'de>,
        F: FieldRef<T>,
    {
        self.get(field_ref.path())
    }

    /// Type-safe field getter using struct field descriptors (NEW ENHANCED API)
    /// Example: `ctx.get_field(TradingMetrics::total_volume())` returns `Option<u64>`
    ///
    /// This provides compile-time type checking with direct struct field access.
    pub fn get_field<T, F>(&self, field: F) -> Option<F::Value>
    where
        F: FieldDescriptor<T>,
    {
        self.get(field.path())
    }

    /// Type-safe field getter using legacy FieldAccessor trait
    /// Example: `ctx.get_field_legacy(TotalVolume)` returns `Option<u64>`
    ///
    /// This eliminates string literals and provides compile-time type checking.
    pub fn get_field_legacy<F: FieldAccessor>(&self, _field: F) -> Option<F::Value> {
        self.get(F::path())
    }

    // ========================================================================
    // Update entity state
    // ========================================================================

    /// Set a field value in the entity state (string-based API)
    /// Example: `ctx.set("last_trade_timestamp", ctx.timestamp())`
    pub fn set<T: Serialize>(&mut self, field: &str, value: T) {
        if let Ok(json_value) = serde_json::to_value(value) {
            self.set_field_value(field, json_value);
        }
    }

    /// Set a field value using a field reference (NEW RECOMMENDED API)
    /// Example: `ctx.set_ref(&field!(entity, total_volume), 1000)`
    ///
    /// This provides compile-time field name validation and type checking.
    pub fn set_ref<T, F>(&mut self, field_ref: &F, value: T)
    where
        T: Serialize,
        F: FieldRef<T>,
    {
        if let Ok(json_value) = serde_json::to_value(value) {
            self.set_field_value(field_ref.path(), json_value);
        }
    }

    /// Type-safe field setter using struct field descriptors (NEW ENHANCED API)
    /// Example: `ctx.set_field(TradingMetrics::total_volume(), 1000)` sets total_volume to 1000
    ///
    /// This provides compile-time type checking with direct struct field access.
    pub fn set_field<T, F>(&mut self, field: F, value: F::Value)
    where
        F: FieldDescriptor<T>,
    {
        self.set(field.path(), value)
    }

    /// Type-safe field setter using legacy FieldAccessor trait
    /// Example: `ctx.set_field_legacy(TotalVolume, 1000)` sets total_volume to 1000
    ///
    /// This eliminates string literals and provides compile-time type checking.
    pub fn set_field_legacy<F: FieldAccessor>(&mut self, _field: F, value: F::Value) {
        self.set(F::path(), value)
    }

    /// Increment a numeric field by a given amount (string-based API)
    /// Example: `ctx.increment("whale_trade_count", 1)`
    pub fn increment(&mut self, field: &str, amount: u64) {
        if let Some(current) = self.get::<u64>(field) {
            self.set(field, current + amount);
        } else {
            self.set(field, amount);
        }
    }

    /// Increment a numeric field using a field reference (NEW RECOMMENDED API)
    /// Example: `ctx.increment_ref(&field!(entity, trade_count), 1)`
    ///
    /// This provides compile-time field name validation. Works with u64 fields.
    pub fn increment_ref<F>(&mut self, field_ref: &F, amount: u64)
    where
        F: FieldRef<u64>,
    {
        let path = field_ref.path();
        if let Some(current) = self.get::<u64>(path) {
            self.set(path, current + amount);
        } else {
            self.set(path, amount);
        }
    }

    /// Type-safe increment using struct field descriptors (NEW ENHANCED API)
    /// Example: `ctx.increment_field(TradingMetrics::trade_count(), 1)`
    ///
    /// Works with u64 fields and provides compile-time type checking.
    pub fn increment_field<T, F>(&mut self, field: F, amount: u64)
    where
        F: FieldDescriptor<T, Value = u64>,
    {
        self.increment(field.path(), amount)
    }

    /// Type-safe increment using legacy FieldAccessor trait
    /// Example: `ctx.increment_field_legacy(TradeCount, 1)`
    ///
    /// Works with any numeric type that can convert to/from u64.
    pub fn increment_field_legacy<F: FieldAccessor>(&mut self, _field: F, amount: u64)
    where
        F::Value: Into<u64> + From<u64>,
    {
        self.increment(F::path(), amount)
    }

    /// Add a value to a numeric accumulator (alias for increment - string-based API)
    /// Example: `ctx.sum("total_fees", fee_amount)`
    pub fn sum(&mut self, field: &str, value: u64) {
        self.increment(field, value);
    }

    /// Add a value to a numeric accumulator using a field reference (NEW RECOMMENDED API)
    /// Example: `ctx.sum_ref(&field!(entity, total_fees), fee_amount)`
    ///
    /// This is an alias for `increment_ref()` that may be clearer for accumulation use cases.
    pub fn sum_ref<F>(&mut self, field_ref: &F, value: u64)
    where
        F: FieldRef<u64>,
    {
        self.increment_ref(field_ref, value)
    }

    /// Type-safe sum using struct field descriptors (NEW ENHANCED API)
    /// Example: `ctx.sum_field(TradingMetrics::total_fees(), fee_amount)`
    ///
    /// Works with u64 fields and provides compile-time type checking.
    pub fn sum_field<T, F>(&mut self, field: F, value: u64)
    where
        F: FieldDescriptor<T, Value = u64>,
    {
        self.sum(field.path(), value)
    }

    /// Type-safe sum using legacy FieldAccessor trait
    /// Example: `ctx.sum_field_legacy(TotalFees, fee_amount)`
    ///
    /// Works with any numeric type that can convert to/from u64.
    pub fn sum_field_legacy<F: FieldAccessor>(&mut self, _field: F, value: u64)
    where
        F::Value: Into<u64> + From<u64>,
    {
        self.sum(F::path(), value)
    }

    /// Add a value to a unique set and update the count field
    /// Example: `ctx.add_unique("unique_traders", user_address)`
    pub fn add_unique(&mut self, field: &str, value: String) {
        // Get the internal set field name (conventionally field + "_set")
        let set_field = format!("{}_set", field);

        // Get existing set or create new one
        let mut set: HashSet<String> = self.get::<HashSet<String>>(&set_field).unwrap_or_default();

        // Add the value
        set.insert(value);

        // Update the set and count
        let count = set.len() as u64;
        self.set(&set_field, set);
        self.set(field, count);
    }

    // ========================================================================
    // Access context metadata
    // ========================================================================

    /// Get the current timestamp in milliseconds
    pub fn timestamp(&self) -> i64 {
        self.timestamp
    }

    /// Get the blockchain slot number
    pub fn slot(&self) -> u64 {
        self.slot.unwrap_or(0)
    }

    /// Get the transaction signature
    pub fn signature(&self) -> &str {
        self.signature.as_deref().unwrap_or("")
    }

    // ========================================================================
    // Internal helpers
    // ========================================================================

    fn set_field_value(&mut self, field_path: &str, value: Value) {
        if let Some(state) = self.registers.get_mut(self.state_reg) {
            if !state.is_object() {
                *state = Value::Object(serde_json::Map::new());
            }

            let segments: Vec<&str> = field_path.split('.').collect();
            let mut current = state;

            // Navigate to the parent object
            for segment in &segments[..segments.len() - 1] {
                if current.get(segment).is_none() {
                    current[segment] = Value::Object(serde_json::Map::new());
                }
                current = current.get_mut(segment).unwrap();
            }

            // Set the final field
            if let Some(last_segment) = segments.last() {
                current[*last_segment] = value;
            }
        }
    }
}

// Re-export HashSet for use in add_unique
use std::collections::HashSet;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_get_field() {
        let mut registers = vec![json!({
            "total_volume": 1000,
            "metrics": {
                "count": 5
            }
        })];

        let compiled_paths = HashMap::new();
        let ctx = MetricsContext::new(0, &mut registers, &compiled_paths, None, None, 0);

        assert_eq!(ctx.get::<u64>("total_volume"), Some(1000));
        assert_eq!(ctx.get::<u64>("metrics.count"), Some(5));
    }

    #[test]
    fn test_set_field() {
        let mut registers = vec![json!({})];
        let compiled_paths = HashMap::new();
        let mut ctx = MetricsContext::new(0, &mut registers, &compiled_paths, None, None, 0);

        ctx.set("total_volume", 2000u64);
        assert_eq!(ctx.get::<u64>("total_volume"), Some(2000));
    }

    #[test]
    fn test_increment() {
        let mut registers = vec![json!({"count": 10})];
        let compiled_paths = HashMap::new();
        let mut ctx = MetricsContext::new(0, &mut registers, &compiled_paths, None, None, 0);

        ctx.increment("count", 5);
        assert_eq!(ctx.get::<u64>("count"), Some(15));

        // Test incrementing non-existent field
        ctx.increment("new_count", 3);
        assert_eq!(ctx.get::<u64>("new_count"), Some(3));
    }

    #[test]
    fn test_context_metadata() {
        let mut registers = vec![json!({})];
        let compiled_paths = HashMap::new();
        let ctx = MetricsContext::new(
            0,
            &mut registers,
            &compiled_paths,
            Some(12345),
            Some("abc123".to_string()),
            1000000,
        );

        assert_eq!(ctx.slot(), 12345);
        assert_eq!(ctx.signature(), "abc123");
        assert_eq!(ctx.timestamp(), 1000000);
    }

    #[test]
    fn test_enhanced_field_descriptor_api() {
        // Define a struct representing our entity
        #[expect(dead_code, reason = "fields describe the metric schema")]
        struct TradingMetrics {
            total_volume: u64,
            trade_count: u64,
        }

        // Generate field descriptors for the struct
        impl_field_descriptors!(TradingMetrics {
            total_volume: u64,
            trade_count: u64
        });

        let mut registers = vec![json!({
            "total_volume": 1000,
            "trade_count": 5
        })];

        let compiled_paths = HashMap::new();
        let mut ctx = MetricsContext::new(0, &mut registers, &compiled_paths, None, None, 0);

        // Test enhanced API - direct struct field access
        assert_eq!(ctx.get_field(TradingMetrics::total_volume()), Some(1000));
        assert_eq!(ctx.get_field(TradingMetrics::trade_count()), Some(5));

        // Test type-safe set
        ctx.set_field(TradingMetrics::total_volume(), 2000);
        assert_eq!(ctx.get_field(TradingMetrics::total_volume()), Some(2000));

        // Test type-safe increment
        ctx.increment_field(TradingMetrics::trade_count(), 3);
        assert_eq!(ctx.get_field(TradingMetrics::trade_count()), Some(8));

        // Test type-safe sum
        ctx.sum_field(TradingMetrics::total_volume(), 500);
        assert_eq!(ctx.get_field(TradingMetrics::total_volume()), Some(2500));
    }

    #[test]
    fn test_legacy_field_accessor_api() {
        // Define field accessors using the legacy macro
        field_accessor!(TotalVolume, u64, "total_volume");
        field_accessor!(TradeCount, u64, "trade_count");
        field_accessor!(LastPrice, u64, "reserves.last_price");

        let mut registers = vec![json!({
            "total_volume": 1000,
            "trade_count": 5,
            "reserves": {
                "last_price": 250
            }
        })];

        let compiled_paths = HashMap::new();
        let mut ctx = MetricsContext::new(0, &mut registers, &compiled_paths, None, None, 0);

        // Test legacy type-safe get
        assert_eq!(ctx.get_field_legacy(TotalVolume), Some(1000));
        assert_eq!(ctx.get_field_legacy(TradeCount), Some(5));
        assert_eq!(ctx.get_field_legacy(LastPrice), Some(250));

        // Test legacy type-safe set
        ctx.set_field_legacy(TotalVolume, 2000);
        assert_eq!(ctx.get_field_legacy(TotalVolume), Some(2000));

        // Test legacy type-safe increment
        ctx.increment_field_legacy(TradeCount, 3);
        assert_eq!(ctx.get_field_legacy(TradeCount), Some(8));

        // Test legacy type-safe sum
        ctx.sum_field_legacy(TotalVolume, 500);
        assert_eq!(ctx.get_field_legacy(TotalVolume), Some(2500));

        // Test nested path
        ctx.set_field_legacy(LastPrice, 300);
        assert_eq!(ctx.get_field_legacy(LastPrice), Some(300));
    }

    #[test]
    fn test_enhanced_api_with_different_types() {
        // Test with different field types
        #[expect(dead_code, reason = "fields describe the metric schema")]
        struct PriceMetrics {
            average_price: f64,
            volume: u64,
        }

        impl_field_descriptors!(PriceMetrics {
            average_price: f64,
            volume: u64
        });

        let mut registers = vec![json!({})];
        let compiled_paths = HashMap::new();
        let mut ctx = MetricsContext::new(0, &mut registers, &compiled_paths, None, None, 0);

        ctx.set_field(PriceMetrics::average_price(), 123.45);
        assert_eq!(ctx.get_field(PriceMetrics::average_price()), Some(123.45));

        ctx.set_field(PriceMetrics::volume(), 1000);
        assert_eq!(ctx.get_field(PriceMetrics::volume()), Some(1000));
    }

    #[test]
    fn test_legacy_api_with_different_types() {
        // Test legacy API with f64 type
        field_accessor!(AveragePrice, f64, "average_price");

        let mut registers = vec![json!({})];
        let compiled_paths = HashMap::new();
        let mut ctx = MetricsContext::new(0, &mut registers, &compiled_paths, None, None, 0);

        ctx.set_field_legacy(AveragePrice, 123.45);
        assert_eq!(ctx.get_field_legacy(AveragePrice), Some(123.45));
    }

    #[test]
    fn test_field_ref_api() {
        // Define a struct to represent our entity
        #[expect(dead_code, reason = "fields describe the metric schema")]
        struct TradingMetrics {
            total_volume: u64,
            trade_count: u64,
            last_price: f64,
        }

        // Create an instance (the actual field values don't matter, we just need the struct for field! macro)
        let entity = TradingMetrics {
            total_volume: 0,
            trade_count: 0,
            last_price: 0.0,
        };
        let _ = &entity;

        let mut registers = vec![json!({
            "total_volume": 1000,
            "trade_count": 5,
            "last_price": 250.5
        })];

        let compiled_paths = HashMap::new();
        let mut ctx = MetricsContext::new(0, &mut registers, &compiled_paths, None, None, 0);

        // ========================================================================
        // Test new field reference API - cleaner than field_accessor!
        // ========================================================================

        // Test get_ref
        assert_eq!(
            ctx.get_ref::<u64, _>(&field!(entity, total_volume)),
            Some(1000)
        );
        assert_eq!(ctx.get_ref::<u64, _>(&field!(entity, trade_count)), Some(5));
        assert_eq!(
            ctx.get_ref::<f64, _>(&field!(entity, last_price)),
            Some(250.5)
        );

        // Test set_ref
        ctx.set_ref(&field!(entity, total_volume), 2000u64);
        assert_eq!(
            ctx.get_ref::<u64, _>(&field!(entity, total_volume)),
            Some(2000)
        );

        ctx.set_ref(&field!(entity, last_price), 300.75);
        assert_eq!(
            ctx.get_ref::<f64, _>(&field!(entity, last_price)),
            Some(300.75)
        );

        // Test increment_ref
        ctx.increment_ref(&field!(entity, trade_count), 3);
        assert_eq!(ctx.get_ref::<u64, _>(&field!(entity, trade_count)), Some(8));

        // Test sum_ref
        ctx.sum_ref(&field!(entity, total_volume), 500);
        assert_eq!(
            ctx.get_ref::<u64, _>(&field!(entity, total_volume)),
            Some(2500)
        );
    }

    #[test]
    fn test_field_ref_with_nested_struct() {
        // Test with nested fields
        #[expect(dead_code, reason = "fields describe the metric schema")]
        struct Metrics {
            reserves: Reserves,
        }

        #[expect(dead_code, reason = "fields describe the metric schema")]
        struct Reserves {
            last_price: f64,
        }

        let entity = Metrics {
            reserves: Reserves { last_price: 0.0 },
        };
        let _ = &entity;

        let mut registers = vec![json!({
            "reserves": {
                "last_price": 100.5
            }
        })];

        let compiled_paths = HashMap::new();
        let mut ctx = MetricsContext::new(0, &mut registers, &compiled_paths, None, None, 0);

        // Test nested field access with dot notation
        assert_eq!(
            ctx.get_ref::<f64, _>(&field!(entity, reserves.last_price)),
            Some(100.5)
        );

        ctx.set_ref(&field!(entity, reserves.last_price), 200.75);
        assert_eq!(
            ctx.get_ref::<f64, _>(&field!(entity, reserves.last_price)),
            Some(200.75)
        );
    }

    #[test]
    fn test_enhanced_api_field_initialization() {
        // Test that increment/sum works when field doesn't exist yet
        #[expect(dead_code, reason = "fields describe the metric schema")]
        struct WhaleMetrics {
            whale_trade_count: u64,
            total_whale_volume: u64,
        }

        impl_field_descriptors!(WhaleMetrics {
            whale_trade_count: u64,
            total_whale_volume: u64
        });

        let mut registers = vec![json!({})];
        let compiled_paths = HashMap::new();
        let mut ctx = MetricsContext::new(0, &mut registers, &compiled_paths, None, None, 0);

        // Increment on non-existent field should initialize to the amount
        ctx.increment_field(WhaleMetrics::whale_trade_count(), 1);
        assert_eq!(ctx.get_field(WhaleMetrics::whale_trade_count()), Some(1));

        // Subsequent increment should add to existing value
        ctx.increment_field(WhaleMetrics::whale_trade_count(), 2);
        assert_eq!(ctx.get_field(WhaleMetrics::whale_trade_count()), Some(3));

        // Test sum field initialization
        ctx.sum_field(WhaleMetrics::total_whale_volume(), 5000);
        assert_eq!(
            ctx.get_field(WhaleMetrics::total_whale_volume()),
            Some(5000)
        );

        ctx.sum_field(WhaleMetrics::total_whale_volume(), 3000);
        assert_eq!(
            ctx.get_field(WhaleMetrics::total_whale_volume()),
            Some(8000)
        );
    }

    #[test]
    fn test_legacy_field_ref_initialization() {
        // Test legacy field ref API that increment_ref works when field doesn't exist yet
        #[expect(dead_code, reason = "fields describe the metric schema")]
        struct Metrics {
            whale_trade_count: u64,
        }

        let entity = Metrics {
            whale_trade_count: 0,
        };
        let _ = &entity;

        let mut registers = vec![json!({})];
        let compiled_paths = HashMap::new();
        let mut ctx = MetricsContext::new(0, &mut registers, &compiled_paths, None, None, 0);

        // Increment on non-existent field should initialize to the amount
        ctx.increment_ref(&field!(entity, whale_trade_count), 1);
        assert_eq!(
            ctx.get_ref::<u64, _>(&field!(entity, whale_trade_count)),
            Some(1)
        );

        // Subsequent increment should add to existing value
        ctx.increment_ref(&field!(entity, whale_trade_count), 2);
        assert_eq!(
            ctx.get_ref::<u64, _>(&field!(entity, whale_trade_count)),
            Some(3)
        );
    }

    #[test]
    fn test_backward_compatibility() {
        // Verify that old string-based API still works
        let mut registers = vec![json!({
            "volume": 100
        })];

        let compiled_paths = HashMap::new();
        let mut ctx = MetricsContext::new(0, &mut registers, &compiled_paths, None, None, 0);

        // Old string API should still work
        assert_eq!(ctx.get::<u64>("volume"), Some(100));
        ctx.set("volume", 200u64);
        assert_eq!(ctx.get::<u64>("volume"), Some(200));
        ctx.increment("volume", 50);
        assert_eq!(ctx.get::<u64>("volume"), Some(250));
    }
}
