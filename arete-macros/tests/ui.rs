#[test]
fn ui() {
    let tests = trybuild::TestCases::new();
    tests.pass("tests/ui/pass/*.rs");
    tests.compile_fail("tests/ui/map_errors/*.rs");
    tests.compile_fail("tests/ui/resolve_errors/*.rs");
    tests.compile_fail("tests/ui/validation_errors/*.rs");
    tests.compile_fail("tests/ui/view_errors/*.rs");
}
