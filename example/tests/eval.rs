use pacm_v8::Isolate;
use std::sync::{Arc, Mutex};
use std::{fs, path::PathBuf};

#[test]
fn evaluates_js_file() {
    let script_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("simple.js");
    let source = fs::read_to_string(&script_path)
        .expect("failed to load fixture JavaScript file");

    let mut isolate = Isolate::new().expect("failed to initialize V8 isolate");
    let mut context = isolate
        .create_context()
        .expect("failed to create V8 execution context");

    let result = context
        .eval(&source)
        .expect("JavaScript execution returned null");

    assert_eq!(result.as_str(), "PACM:42");

    context.dispose();
    drop(context);
    // Explicitly dispose to make it obvious the shim manages lifetime.
    isolate.dispose();
}

#[test]
fn registers_native_function() {
    let mut isolate = Isolate::new().expect("failed to initialize V8 isolate");
    let mut context = isolate
        .create_context()
        .expect("failed to create V8 execution context");

    let calls: Arc<Mutex<Vec<Vec<String>>>> = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&calls);
    context
        .add_function("console.log", move |args| {
            let mut guard = captured.lock().expect("failed to lock log storage");
            guard.push(args.iter().map(|value| value.as_str().to_owned()).collect());
            Ok(None)
        })
        .expect("failed to register native function");

    context
        .eval("console.log('hello', 'world');")
        .expect("execution failed");

    let guard = calls.lock().expect("failed to lock log storage");
    assert_eq!(guard.len(), 1);
    assert_eq!(guard[0], vec!["hello".to_string(), "world".to_string()]);
}
