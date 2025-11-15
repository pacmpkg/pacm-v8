use pacm_v8::{Context, Isolate, JsValue, Result as V8Result, Script, V8Error};
use std::{env, fs, path::PathBuf, process};

const COMPILED_SNIPPET: &str = r#"
console.log("[compiled] calling host.echo");
const echoed = host.echo("[compiled] Script::run");
console.log("[compiled] host.echo returned:", echoed);

const report = {
    caller: "compiled snippet",
    description: describeHost("compiled snippet"),
    product: jsMultiply("3", "5"),
};

JSON.stringify(report);
"#;

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        process::exit(1);
    }
}

fn run() -> V8Result<()> {
    let script_path = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("example")
                .join("src")
                .join("fixtures")
                .join("simple.js")
        });

    let bootstrap_source =
        fs::read_to_string(&script_path).map_err(|err| V8Error::new(err.to_string()))?;

    let mut isolate = Isolate::new()?;
    println!("[Rust] isolate handle: {:?}", isolate.raw_handle());

    let mut context = isolate.create_context()?;
    println!("[Rust] context handle: {:?}", context.raw_handle());
    println!("[Rust] context isolate handle: {:?}", context.isolate_handle());
    debug_assert_eq!(context.isolate_handle(), isolate.raw_handle());

    install_host_bindings(&mut context)?;
    prime_js_globals(&context)?;

    let bootstrap_result = context.eval(&bootstrap_source)?;
    println!("[Rust] Context::eval -> {}", bootstrap_result);

    let description = context.call_function("describeHost", &["Rust entry point"])?;
    println!("[Rust] Context::call_function -> {}", description.as_str());

    let mut compiled = Script::compile(&isolate, COMPILED_SNIPPET)?;
    println!("[Rust] script handle: {:?}", compiled.raw_handle());

    let summary = compiled.run(&context)?;
    let summary_json = summary.into_string();
    println!("[Rust] Script::run -> {summary_json}");

    compiled.dispose();
    context.dispose();
    isolate.dispose();

    Ok(())
}

fn prime_js_globals(context: &Context) -> V8Result<()> {
    context.set_global_str("greeting", "Hello from Rust!")?;
    context.set_global_str("host.info.name", "pacm-v8 bindings")?;
    context.set_global_number("host.info.version", 1.0)?;
    context.set_global_number("host.info.multiplier", 2.0)?;
    Ok(())
}

fn install_host_bindings(context: &mut Context) -> V8Result<()> {
    context.add_function("console.log", |args| -> V8Result<Option<JsValue>> {
        let message = args
            .iter()
            .map(|value| value.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        println!("[console.log] {message}");
        Ok(None)
    })?;

    context.add_function("host.echo", |args| -> V8Result<Option<JsValue>> {
        let payload = args
            .iter()
            .map(|value| value.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        println!("[host.echo] {payload}");
        Ok(args.first().cloned())
    })?;

    Ok(())
}
