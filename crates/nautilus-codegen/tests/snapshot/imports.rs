use super::support::{generated_named_file, generated_python_file, validate};
use nautilus_codegen::generator::generate_all_models;
use nautilus_codegen::js::generate_all_js_models;
use nautilus_codegen::python::generate_all_python_models;

/// A schema whose model pulls in several enums, composite types and relations
/// at once — the imports that used to be collected in a `HashSet`.
const MULTI_IMPORT_SCHEMA: &str = r#"
enum Status { Active Inactive }
enum Role { Admin Member }
enum Tier { Free Pro }

type Address {
  street String
  city   String
}

type Contact {
  email String
  phone String
}

model User {
  id      Int      @id @default(autoincrement())
  status  Status
  role    Role
  tier    Tier
  address Address
  contact Contact
  posts   Post[]
  notes   Note[]
}

model Post {
  id       Int  @id @default(autoincrement())
  authorId Int
  author   User @relation(fields: [authorId], references: [id])
}

model Note {
  id       Int  @id @default(autoincrement())
  authorId Int
  author   User @relation(fields: [authorId], references: [id])
}
"#;

#[test]
fn test_generated_imports_are_stable_across_runs() {
    let ir = validate(MULTI_IMPORT_SCHEMA);

    for _ in 0..8 {
        let rust = generate_all_models(&ir, false).expect("generate_all_models should succeed");
        assert_eq!(
            rust.get("User"),
            generate_all_models(&ir, false)
                .expect("generate_all_models should succeed")
                .get("User"),
            "Rust model output must not vary between runs"
        );

        let python = generate_all_python_models(&ir, false, 3)
            .expect("generate_all_python_models should succeed");
        assert_eq!(
            generated_python_file(&python, "user.py"),
            generated_python_file(
                &generate_all_python_models(&ir, false, 3)
                    .expect("generate_all_python_models should succeed"),
                "user.py"
            ),
            "Python model output must not vary between runs"
        );

        let (js, dts) = generate_all_js_models(&ir).expect("generate_all_js_models should succeed");
        let (js_again, dts_again) =
            generate_all_js_models(&ir).expect("generate_all_js_models should succeed");
        assert_eq!(js, js_again, "JS model output must not vary between runs");
        assert_eq!(dts, dts_again, "d.ts output must not vary between runs");
    }
}

#[test]
fn test_generated_imports_are_sorted() {
    let ir = validate(MULTI_IMPORT_SCHEMA);

    let rust = generate_all_models(&ir, false).expect("generate_all_models should succeed");
    let rust_user = rust.get("User").expect("User model missing");
    assert!(
        rust_user.contains(
            "use super::enums::Role;\nuse super::enums::Status;\nuse super::enums::Tier;"
        ),
        "Rust enum imports should be sorted:\n{rust_user}"
    );
    assert!(
        rust_user.contains("use super::types::Address;\nuse super::types::Contact;"),
        "Rust composite type imports should be sorted:\n{rust_user}"
    );
    assert!(
        rust_user.contains("use super::Note;\nuse super::Post;"),
        "Rust relation imports should be sorted:\n{rust_user}"
    );

    let python = generate_all_python_models(&ir, false, 3)
        .expect("generate_all_python_models should succeed");
    let python_user = generated_python_file(&python, "user.py");
    assert!(
        python_user.contains("from ..enums.enums import Role, Status, Tier"),
        "Python enum imports should be sorted:\n{python_user}"
    );
    assert!(
        python_user.contains("from ..types.types import Address, Contact"),
        "Python composite type imports should be sorted:\n{python_user}"
    );

    let (_, dts) = generate_all_js_models(&ir).expect("generate_all_js_models should succeed");
    let js_user = generated_named_file(&dts, "user.d.ts");
    assert!(
        js_user.contains("import type { Role, Status, Tier } from '../enums.js';"),
        "TypeScript enum imports should be sorted:\n{js_user}"
    );
}
