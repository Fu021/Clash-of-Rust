#[path = "../schema.rs"]
mod schema;

#[test]
fn desktop_contract_reads_shared_models_and_properties_after_nested_callbacks() {
    let state = r#"
        export enum Intent { open, close }
        export struct Choice { id: string }
        export global UiData {
            pure callback blocked() -> bool;
            blocked => { if true { true } else { false } }
            in-out property <string> value: "}";
            // }
            /* { */
            in-out property <[SharedItem]> items;
        }
    "#;
    let models = "export struct SharedItem { title: string, enabled: bool }";
    let parsed = schema::parse(state, models);
    assert_eq!(parsed.intents, ["open", "close"]);
    assert_eq!(
        parsed
            .structs
            .iter()
            .map(|item| item.0.as_str())
            .collect::<Vec<_>>(),
        ["Choice", "SharedItem"]
    );
    assert_eq!(
        parsed
            .properties
            .iter()
            .map(|item| item.0.as_str())
            .collect::<Vec<_>>(),
        ["value", "items"]
    );
    assert_eq!(schema::rust_name("focus-target"), "focus_target");
    assert_eq!(schema::variant("focus-input"), "FocusInput");
    assert_eq!(schema::wire_type("[SharedItem]"), "List<SharedItem>");

    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../client-ui/ui/");
    let production = schema::read(
        &format!("{root}state.slint"),
        &format!("{root}models.slint"),
    );
    assert!(production.structs.iter().any(|item| item.0 == "GroupItem"));
    assert!(
        production
            .properties
            .iter()
            .any(|item| item.0 == "focus-revision")
    );
}
