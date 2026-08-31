# Nautilus Schema Language Grammar

This document defines the complete grammar for the schema language used in Nautilus. The grammar is specified in Extended Backus-Naur Form (EBNF).

## Notation

- `::=` means "is defined as"
- `|` separates alternatives
- `[ ... ]` indicates optional elements (zero or one)
- `{ ... }` indicates repetition (zero or more)
- `( ... )` groups elements
- `'...'` denotes literal terminals (keywords, operators)
- `"..."` denotes string literals
- `Ident`, `String`, `Number` are terminal tokens from the lexer

## Schema Structure

### Top-Level

```ebnf
Schema ::= Declaration* EOF

Declaration ::= ImportDecl
              | DatasourceDecl
              | GeneratorDecl  
              | ModelDecl
              | ViewDecl
              | TypeDecl
              | EnumDecl

Newline ::= '\n' | '\r\n'
```

### Multiple Files

A schema may be split across several `.nautilus` files in one directory. Point
`--schema` at the directory instead of at a file and every `.nautilus` file
directly inside it is assembled, in lexicographic filename order, into a single
schema:

```
schema/
  00-datasource.nautilus   datasource + generator
  enums.nautilus           enum Role { ... }
  user.nautilus            model User { role Role ... posts Post[] }
  post.nautilus            model Post { author User @relation(...) }
```

```bash
nautilus generate --schema ./schema
```

Order affects nothing but the assembled text: every reference is resolved by
name across the whole set, so a model may reference an enum or another model
declared in any file. The usual whole-schema rules still apply to the set as a
whole — one `datasource`, one `generator`, no duplicate declaration names.

Diagnostics report the file, line and column the developer wrote. `nautilus
format` given a directory formats each file separately rather than merging them.

Pointing `--schema` at a single file loads that file and everything it imports.

### Imports

```ebnf
ImportDecl ::= 'import' String Newline*
```

An `import` joins other schema declarations to this one. The path is relative
to the directory of the file that declares it and may name either a
`.nautilus` file or a directory containing `.nautilus` files:

```prisma
import "./enums.nautilus"
import "../shared"

model User {
  id    Int    @id @default(autoincrement())
  role  Role
  posts Post[]
}
```

Imports are followed transitively, a file reached twice is joined once, and
cycles are allowed (`a` importing `b` importing `a` loads both files once).
Importing is not scoping: the assembled set is one flat schema, so `import` says
*which files belong together*, not which names a file may use. A cycle is
therefore not an error but the honest shape of a mutual dependency — two models
with a relation between them each need the other's file.

What a schema may not have twice is a root: **more than one `datasource`, or
more than one `generator`, in the assembled set is an error**, reported on the
second block. That is what catches an import which reached another schema's
root file instead of the shared models it was meant to pull in.

An import naming a path that does not exist, a regular file whose name does not
end in `.nautilus`, or a directory containing no `.nautilus` files is an error
reported on the `import` line. The files that were reached are still validated,
so one broken path does not hide the rest of the schema.

This is also how an editor knows a file is part of a larger schema. The language
server assembles the open file with everything it imports before analysing it,
so a reference across files resolves and a diagnostic lands on the file that
holds it. Completion inside the quoted import path reads the filesystem and
offers directories as import targets or for navigation, plus `.nautilus` files.
A file that imports nothing is analysed on its own — sibling files in
the same directory are *not* joined implicitly, which is what keeps a directory
of alternative schemas (one per provider, say) from being merged into a pile of
duplicate declarations.

Every declaration except `import` may appear in any file of the set; the
whole-schema rules (one `datasource`, one `generator`, no duplicate names) apply
to the assembled set.

## Declarations

### Datasource

```ebnf
DatasourceDecl ::= 'datasource' Ident '{' Newline*
                   ConfigField*
                   '}' Newline*

ConfigField ::= Ident '=' Expr Newline*
```

**Example:**
```prisma
datasource db {
  provider = "postgresql"
  url      = env("DATABASE_URL")
  extensions = [pg_trgm, "uuid-ossp"]
  preserve_extensions = true
}
```

**Fields:**

| Field | Required | Values | Default | Description |
|-------|----------|--------|---------|-------------|
| `provider` | ✓ | `"postgresql"` \| `"mysql"` \| `"sqlite"` | — | Database provider |
| `url` | ✓ | string literal or `env("VAR")` | — | Runtime database connection URL |
| `direct_url` | — | string literal or `env("VAR")` | — | Direct admin/introspection URL |
| `extensions` | — | array of identifiers/string literals | `[]` | PostgreSQL-only extensions to install via `CREATE EXTENSION IF NOT EXISTS` before type/table DDL |
| `preserve_extensions` | — | boolean literal | `false` | PostgreSQL-only; preserve live extensions that are not listed in `extensions` instead of diffing them as drops |

The `extensions` field is only valid when `provider = "postgresql"`. It accepts
both bare identifiers such as `pg_trgm` and quoted names such as `"uuid-ossp"`.
The `preserve_extensions` field is also PostgreSQL-only. When it is `true`,
Nautilus still creates missing declared extensions but does not propose dropping
extra live extensions.

### Generator

```ebnf
GeneratorDecl ::= 'generator' Ident '{' Newline*
                  ConfigField*
                  '}' Newline*
```

**Fields:**

| Field | Required | Values | Default | Description |
|-------|----------|--------|---------|-------------|
| `provider` | ✓ | `"nautilus-client-rs"` \| `"nautilus-client-py"` \| `"nautilus-client-js"` | — | Code-generation target language |
| `output` | — | string path | — | Output directory for generated files |
| `interface` | — | `"sync"` \| `"async"` | `"sync"` | Whether to generate a synchronous or asynchronous client API |
| `recursive_type_depth` | — | positive integer | `5` | Depth of recursive include TypedDicts (Python client only) |

The `interface` fields:
- `"sync"` (default): generates plain `fn` / `def` methods. Rust delegates use
  `tokio::task::block_in_place`; Python delegates use `asyncio.run()`.
- `"async"`: generates `async fn` / `async def` methods with `.await` / `await`
  at every call-site.

The `recursive_type_depth` field controls how many levels of nested `include` TypedDicts
are generated for the Python client:
- Each depth level adds a `{Model}IncludeRecursive{N}` type and the corresponding
  `FindMany{Target}ArgsFrom{Source}Recursive{N}` types.
- At the maximum depth the `include` field is omitted, preventing infinite type recursion.
- Minimum accepted value is `1`. Values of `0` or below are treated as `1`.

**Example:**
```prisma
generator client {
  provider  = "nautilus-client-rs"
  output    = "../generated"
  interface = "async"
}
```

```prisma
generator client {
  provider            = "nautilus-client-py"
  output              = "../generated"
  interface           = "async"
  recursive_type_depth  = 3   # default is 5
}
```

Java also supports a dedicated generator shape:

```prisma
generator client {
  provider    = "nautilus-client-java"
  output      = "../generated-java"
  package     = "com.acme.db"
  group_id    = "com.acme"
  artifact_id = "db-client"
  mode        = "jar"
  interface   = "async"
}
```

The Java-only fields are `package`, `group_id`, `artifact_id`, and `mode`.

### Model

```ebnf
ModelDecl ::= 'model' Ident '{' Newline*
              ( FieldDecl | ModelAttribute Newline* )*
              '}' Newline*

FieldDecl ::= Ident FieldType FieldModifier? FieldAttribute* Newline*

FieldModifier ::= '?' | '!' | '[' ']'

ModelAttribute ::= '@@' AttributeName AttributeArgs?
```

**Example:**
```prisma
model User {
  id        Int      @id @default(autoincrement())
  email     String!  @unique
  posts     Post[]
  profile   Profile?

  @@map("users")
  @@index([email])
}
```

### View

```ebnf
ViewDecl ::= 'view' Ident '{' Newline*
             ( FieldDecl | ModelAttribute Newline* )*
             '}' Newline*
```

A view has a model's body and parses into the same node, flagged as a view. It
names a read-only relation the database owns: Nautilus queries it and emits no
DDL for it, so the attributes that describe storage — `@@index`, `@@check`,
`@default`, `@updatedAt`, `@computed`, `@check` — are rejected on one, and a
view can take part in no relation, on either side, because it carries no
foreign key.

**Example:**
```prisma
view PublishedPost {
  id     Int    @id
  title  String
  views  Int
  author String

  @@map("published_posts")
}
```

### Composite Type

```ebnf
TypeDecl ::= 'type' Ident '{' Newline*
             FieldDecl*
             '}' Newline*
```

Composite `type` blocks define reusable embedded structures.

```prisma
type Address {
  street String
  zip    String      @map("zip_code")
  kind   AddressKind

  @@map("address_t")
}
```

**Constraints:**
- Fields inside `type` blocks may be scalar or enum types
- Nested composite types and model relations are not allowed
- Only `@map` and `@store(...)` are allowed on composite-type fields
- `@map("name")` renames the field inside the SQL composite type
- `@@map("name")` renames the SQL composite type itself (the only type-level
  attribute allowed); without it the type name is lower-cased

### Enum

```ebnf
EnumDecl ::= 'enum' Ident '{' Newline*
             EnumVariant*
             '}' Newline*

EnumVariant ::= Ident Newline*
```

**Example:**
```prisma
enum Role {
  USER
  ADMIN
  MODERATOR
}
```

## Types

### Field Types

```ebnf
FieldType ::= ScalarType
            | DecimalType
            | UserType

ScalarType ::= 'String'
             | 'Boolean'
             | 'Int'
             | 'BigInt'
             | 'Float'
             | 'DateTime'
             | 'Bytes'
             | 'Json'
             | 'Citext'
             | 'Hstore'
             | 'Ltree'
             | 'Geometry'
             | 'Geography'
             | 'Vector' '(' Number ')'
             | 'Jsonb'
             | 'Uuid'
             | 'Xml'
             | 'Char' '(' Number ')'
             | 'VarChar' '(' Number ')'

DecimalType ::= 'Decimal' '(' Number ',' Number ')'

UserType ::= Ident  // Reference to model or enum
```

**Examples:**
```prisma
field1  String        // Scalar type (implicitly NOT NULL)
field2  Decimal(10,2) // Decimal with precision and scale
field3  Role          // User-defined enum
field4  Post          // User-defined model
field5  String?       // Optional scalar (nullable)
field6  String!       // Explicitly NOT NULL scalar
field7  Post[]        // Array of models
field8  Jsonb         // PostgreSQL-only JSONB
field9  VarChar(255)  // Bounded string
field10 Citext        // PostgreSQL citext extension
field11 Hstore        // PostgreSQL hstore extension
field12 Ltree         // PostgreSQL ltree extension
field13 Geometry      // PostgreSQL PostGIS geometry
field14 Geography     // PostgreSQL PostGIS geography
field15 Vector(1536)  // PostgreSQL pgvector extension
```

## Attributes

### Field Attributes

```ebnf
FieldAttribute ::= '@' AttributeName AttributeArgs?

AttributeName ::= 'id'
                | 'unique'
                | 'default'
                | 'map'
                | 'store'
                | 'relation'
                | 'updatedAt'
                | 'computed'
                | 'check'

AttributeArgs ::= '(' ArgumentList? ')'

ArgumentList ::= Argument ( ',' Argument )*

Argument ::= Expr                    // Positional argument
           | Ident ':' Expr          // Named argument

ComputedKind ::= 'Stored' | 'Virtual'

RawExpr ::= Token+   // Raw SQL tokens, parsed until top-level comma
```

**Recognized Field Attributes:**

#### @id
Marks field as primary key.

```prisma
id Int @id
```

#### @unique
Adds unique constraint.

```prisma
email String @unique
```

#### @default(expr)
Specifies default value.

```prisma
createdAt DateTime @default(now())
count     Int      @default(0)
id        Int      @default(autoincrement())
uuid      Uuid     @default(uuid())
uuidV7    Uuid     @default(uuidv7()) // PostgreSQL
role      String   @default("USER")
active    Boolean  @default(true)
```

#### @map("name")
Maps to physical database column name.

```prisma
userId Int @map("user_id")
```

#### @store(json | native)
Controls how array and composite-type fields are stored when provider capabilities differ.

```prisma
tags    String[] @store(json)
profile Address  @store(json)
```

- `json` — serialize into a JSON column/value
- `native` — use the provider's native array/composite representation when supported

#### @updatedAt
Marks a `DateTime` field to be automatically set to the current timestamp on every CREATE and UPDATE operation. The framework manages this value — it is excluded from all user-input types.

```prisma
updatedAt DateTime @updatedAt
```

#### @computed(expr, Stored | Virtual)
Declares a database-generated (computed) column. The expression is raw SQL evaluated by the database engine.

```prisma
total     Int    @computed(price * quantity, Stored)
fullName  String @computed(first_name || ' ' || last_name, Virtual)
```

- `Stored` — value is computed on write and persisted physically on disk
- `Virtual` — value is computed on read and never stored (not supported on PostgreSQL)

Maps to SQL:
- **PostgreSQL**: `GENERATED ALWAYS AS (expr) STORED`
- **MySQL**: `GENERATED ALWAYS AS (expr) STORED|VIRTUAL`
- **SQLite**: `AS (expr) STORED|VIRTUAL`

**Constraints:**
- Cannot be combined with `@id`, `@default`, or `@updatedAt`
- Cannot be applied to array (`[]`) or relation fields
- `Virtual` is a validation error when the datasource provider is `postgresql`
- Computed fields are read-only — excluded from all create/update input types

#### @check(expr)
Adds a column-level SQL `CHECK` constraint.

```prisma
age    Int    @check(age >= 0 AND age <= 150)
status Status @check(status IN [ACTIVE, PENDING])
```

A predicate the expression language does not cover — `IS NULL`, `LIKE`, a
function call, a typed literal, a provider-specific operator — is written as a
single quoted string and handed to the database verbatim:

```prisma
notes  String?  @check("notes IS NULL OR length(notes) > 3")
```

`db pull` uses this form for every live `CHECK` it cannot express structurally,
so a pulled schema always parses again. Nothing inside a raw predicate is
resolved, so its identifiers are **database column names**, not logical field
names, and neither validation nor `@map` applies to them.

**Constraints:**
- Field-level `@check` may only reference the decorated field itself
- It cannot be applied to relation, array, or computed fields

#### @ignore
Declares that the column exists in the database but Nautilus does not manage it.
The field is left out of the generated client and out of every migration: it is
never created, altered, or dropped, and neither are the indexes, foreign keys
and `CHECK` constraints that reference it.

```prisma
model Device {
  id     Int     @id
  name   String
  uptime String? @ignore
}
```

`db pull` emits `@ignore` on any column whose database type has no Nautilus
spelling, which is what keeps a pulled schema safe to push back: without it the
column would round-trip as `String` and the next `db push` would propose
rewriting it.

**Constraints:**
- Cannot be combined with `@id`, `@unique` or `@relation`, and cannot appear in
  `@@id`, `@@unique` or `@@index` — an unmanaged column is not part of the
  table's shape as Nautilus knows it.
- A required field with no `@default` cannot be ignored on its own: no `create`
  could ever supply it. Give it a default, make it optional, or mark the whole
  model `@@ignore`.

#### @relation(...)
Defines relationship with named arguments. The relation name can also be supplied as the first positional string argument. The `name` parameter is optional but required when multiple relations exist between the same models.

```prisma
author User @relation(
  name: "AuthoredPosts",
  fields: [authorId],
  references: [id],
  onDelete: Cascade,
  onUpdate: Restrict
)

reviewer User @relation("ReviewedPosts", fields: [reviewerId], references: [id])
```

**Supported parameters:**
- `name` (optional): Unique identifier for the relation, required when multiple relations exist between the same two models. Equivalent shorthand: first positional string argument, e.g. `@relation("AuthoredPosts", ...)`
- `fields`: Array of field names in the current model that form the foreign key
- `references`: Array of field names in the referenced model (must be primary key or unique)
- `onDelete` (optional): Referential action on delete
- `onUpdate` (optional): Referential action on update

**Implicit many-to-many.** A relation declared as an array on *both* sides, with
neither side naming `fields`/`references`, is a many-to-many. Nautilus owns a
join table for it:

```prisma
model Post {
  id   Int   @id @default(autoincrement())
  tags Tag[]
}

model Tag {
  id    Int    @id @default(autoincrement())
  posts Post[]
}
```

The table is called `_<A model>To<B model>` — `_PostToTag` here — or
`_<relation name>` when the relation is named. Its two columns are `A` and `B`,
`A` belonging to whichever side sorts first by `(model, field)`, each typed
after the primary key it points at and cascading on delete and update. It is
created and dropped by migrations like any other table, and it never appears in
a generated client: the two array fields are the only way to read or write the
relation.

Both models need a single-field primary key, since each key has one column to
land in. A model can hold a many-to-many with itself, which is the one case
where the same relation `name` appears twice inside one model:

```prisma
model Post {
  id       Int    @id
  related  Post[] @relation(name: "RelatedPost")
  relating Post[] @relation(name: "RelatedPost")
}
```

### Model Attributes

```ebnf
ModelAttribute ::= '@@' AttributeName AttributeArgs?

AttributeName ::= 'map'
                | 'id'
                | 'unique'
                | 'index'
                | 'check'

IndexArgs ::= IdentArray ( ',' IndexNamedArg )*
IndexNamedArg ::= 'type' ':' IndexType
               | 'name' ':' String
               | 'map'  ':' String
IndexType ::= 'BTree' | 'Hash' | 'Gin' | 'Gist' | 'Brin' | 'FullText'
```

**Recognized Model Attributes:**

#### @@map("name")
Maps to physical database table name.

```prisma
model User {
  id Int @id
  @@map("users")
}
```

#### @@id([field1, field2, ...])
Composite primary key.

```prisma
model UserRole {
  userId Int
  roleId Int
  
  @@id([userId, roleId])
}
```

#### @@unique([field1, field2, ...])
Composite unique constraint.

```prisma
model User {
  email    String
  username String
  
  @@unique([email, username])
}
```

#### @@index([field1, field2, ...], type?, name?, map?, where?)
Database index. Supports optional named arguments:

| Argument | Type | Description |
|---|---|---|
| `type` | Ident | Index access method (see table below). Omit to let the DBMS choose (BTree). |
| `name` | String | Logical developer name (not used in DDL). |
| `map` | String | Physical DDL index name override (default: `idx_{table}_{cols}`). |
| `where` | BoolExpr | Partial-index predicate — index only the rows it matches. PostgreSQL and SQLite only. |

**Supported index types by database:**

| Type | PostgreSQL | MySQL | SQLite |
|---|:---:|:---:|:---:|
| `BTree` (default) | ✅ | ✅ | ✅ |
| `Hash` | ✅ | ✅ (8+) | ❌ |
| `Gin` | ✅ | ❌ | ❌ |
| `Gist` | ✅ | ❌ | ❌ |
| `Brin` | ✅ | ❌ | ❌ |
| `FullText` | ❌ | ✅ | ❌ |

Using an unsupported type for the declared datasource provider is a **validation error**.

```prisma
model Post {
  authorId  Int
  createdAt DateTime
  content   String
  
  @@index([authorId, createdAt])
  @@index([authorId, createdAt], type: BTree, map: "idx_post_author_date")
  @@index([content], type: Gin)
}
```

**Partial indexes.** `where:` takes the same boolean expression language as
`@check` / `@@check` and restricts the index to the matching rows, which keeps
it small when queries only ever touch a subset of the table:

```prisma
model Task {
  id      Int     @id
  done    Boolean
  ownerId Int

  @@index([ownerId], where: done = false, map: "idx_task_open")
}
```

MySQL has no partial index syntax; declaring `where:` against a MySQL
datasource is a validation error rather than a silently widened index.

#### @@check(expr)
Adds a table-level SQL `CHECK` constraint.

```prisma
model Booking {
  startDate DateTime
  endDate   DateTime

  @@check(startDate < endDate)
}
```

Unlike field-level `@check`, the model-level form may reference multiple scalar fields.
It accepts the same raw quoted predicate as `@check`:

```prisma
@@check("upper(code) LIKE 'DEV%'")
```

#### @@ignore
Declares that the table exists in the database but Nautilus does not manage it.
The model is left out of the generated client, and migrations neither create nor
drop the table — it is simply not Nautilus's.

```prisma
model LegacyAudit {
  id   Int    @id
  span String @ignore

  @@map("legacy_audit")
  @@ignore
}
```

`db pull` emits `@@ignore` on a table it cannot model well enough to use: one
whose primary key, or whose required column with no default, has a database type
Nautilus has no spelling for.

A model that is *not* ignored may not declare a relation to one that is; mark the
relation field `@ignore` or drop `@@ignore` from the target.

## Expressions

```ebnf
Expr ::= Literal
       | FunctionCall
       | Array
       | Ident

Literal ::= String
          | Number
          | Boolean

Boolean ::= 'true' | 'false'

FunctionCall ::= Ident '(' ArgumentList? ')'

Array ::= '[' ( Expr ( ',' Expr )* )? ']'
```

**Examples:**
```prisma
"string literal"           // String
42                         // Number
3.14                       // Number
true                       // Boolean
false                      // Boolean
autoincrement()            // Function call
uuid()                     // Function call
uuidv7()                   // Function call (PostgreSQL)
now()                      // Function call
env("DATABASE_URL")        // Function call with argument
[userId]                   // Array with single element
[email, username]          // Array with multiple elements
```

### Named Arguments

Used in `@relation` and potentially other attributes:

```ebnf
NamedArg ::= Ident ':' Expr
```

**Example:**
```prisma
@relation(
  name: "PostAuthor",
  fields: [userId],
  references: [id],
  onDelete: Cascade
)
```

## Referential Actions

Used with `@relation` for foreign key constraints:

```ebnf
ReferentialAction ::= 'Cascade'
                    | 'Restrict'
                    | 'NoAction'
                    | 'SetNull'
                    | 'SetDefault'
```

**Example:**
```prisma
user User @relation(
  fields: [userId],
  references: [id],
  onDelete: Cascade,
  onUpdate: SetNull
)
```

## Lexical Grammar

### Tokens

Reference: See [`token.rs`](src/token.rs) for complete token definitions.

```ebnf
Token ::= Keyword
        | Ident
        | String
        | Number
        | Punctuation
        | Attribute
        | Newline
        | EOF

Keyword ::= 'datasource' | 'generator' | 'model' | 'view' | 'enum'
          | 'type' | 'import' | 'true' | 'false'

Ident ::= [a-zA-Z_][a-zA-Z0-9_]*

String ::= '"' StringChar* '"'
StringChar ::= [^"\\\n] | EscapeSeq
EscapeSeq ::= '\\' ( '"' | 'n' | 't' | 'r' | '\\' )

Number ::= Digit+ ( '.' Digit+ )?
Digit ::= [0-9]

Punctuation ::= '{' | '}' | '[' | ']' | '(' | ')'
              | ',' | ':' | '=' | '?' | '!'
              | '*' | '+' | '-' | '/' | '%'
              | '<' | '>' | '|' | '||'

Attribute ::= '@' | '@@'

Comment ::= LineComment | BlockComment
LineComment ::= '//' [^\n]* '\n'
BlockComment ::= '/*' ( [^*] | '*' [^/] )* '*/'
```

### Whitespace

Whitespace (spaces, tabs) is ignored except for newlines, which are significant for statement termination.

### Comments

Both single-line (`//`) and multi-line (`/* */`) comments are supported and ignored by the parser.

## AST Mapping

Each grammar production maps to an AST node type defined in [`ast.rs`](src/ast.rs):

| Grammar Production | AST Type |
|-------------------|----------|
| `Schema` | [`Schema`](src/ast.rs) |
| `Declaration` | [`Declaration`](src/ast.rs) enum |
| `DatasourceDecl` | [`DatasourceDecl`](src/ast.rs) |
| `GeneratorDecl` | [`GeneratorDecl`](src/ast.rs) |
| `ModelDecl` | [`ModelDecl`](src/ast.rs) |
| `EnumDecl` | [`EnumDecl`](src/ast.rs) |
| `FieldDecl` | [`FieldDecl`](src/ast.rs) |
| `FieldType` | [`FieldType`](src/ast.rs) enum |
| `FieldAttribute` | [`FieldAttribute`](src/ast.rs) enum |
| `ModelAttribute` | [`ModelAttribute`](src/ast.rs) enum |
| `Expr` | [`Expr`](src/ast.rs) enum |
| `Literal` | [`Literal`](src/ast.rs) enum |

## Visitor Pattern

The AST supports traversal via the Visitor pattern. See [`visitor.rs`](src/visitor.rs) for details.

**Example visitor implementation:**

```rust
use nautilus_schema::visitor::{Visitor, walk_model};
use nautilus_schema::ast::*;
use nautilus_schema::Result;

struct ModelCounter {
    count: usize,
}

impl Visitor for ModelCounter {
    fn visit_model(&mut self, model: &ModelDecl) -> Result<()> {
        self.count += 1;
        walk_model(self, model) // Continue traversing
    }
}
```

## Grammar Ambiguities and Precedence

### Statement Termination

Newlines are used to terminate field declarations and configuration fields. Multiple newlines are allowed and ignored.

### Optional vs. Required vs. Not-Null

- Fields without any modifier are implicitly required (NOT NULL in SQL)
- Fields with `!` are **explicitly** NOT NULL — identical SQL/codegen behaviour to no modifier, but self-documenting
- Fields with `?` are optional (nullable — no NOT NULL constraint in SQL, wrapped in `Option<T>` / `T | null`)
- Fields with `[]` are arrays (one-to-many relations or lists)
- `!` cannot be used on relation fields (NOT NULL is a column-level constraint and relations have no column)

### User Types vs. Keywords

Field type names like `String`, `Int`, etc., are treated as keywords in type position. Other identifiers in type position are treated as references to user-defined models or enums.

## Error Recovery

The parser implements error recovery at declaration boundaries. If a parse error occurs within a declaration, the parser will:

1. Report the error
2. Skip tokens until the next declaration keyword (`datasource`, `generator`, `model`, `enum`)
3. Continue parsing

This allows multiple errors to be reported in a single parse run.

## Examples

### Complete Schema Example

```prisma
datasource db {
  provider = "postgresql"
  url      = env("DATABASE_URL")
  extensions = [pgcrypto, "uuid-ossp"]
  preserve_extensions = true
}

generator client {
  provider = "nautilus-client-rs"
  output   = "../generated"
}

enum Role {
  USER
  ADMIN
}

model User {
  id        Uuid     @id @default(uuid()) @map("user_id")
  email     String   @unique
  role      Role     @default(USER)
  createdAt DateTime @default(now()) @map("created_at")
  
  posts     Post[]
  
  @@map("users")
}

model Post {
  id        BigInt   @id @default(autoincrement())
  userId    Uuid     @map("user_id")
  title     String
  rating    Decimal(10, 2)
  published Boolean  @default(false)
  createdAt DateTime @default(now())
  
  user      User     @relation(
    fields: [userId],
    references: [id],
    onUpdate: Cascade,
    onDelete: Cascade
  )
  
  @@map("posts")
  @@index([userId, createdAt])
}
```

## Validation

This grammar specifies only **syntax**. Semantic validation (checking that referenced models exist, types are valid, etc.) is performed in Phase 9.1.4 and is not part of the parser.

The parser produces a syntax-valid AST even if the schema has semantic errors like:
- References to non-existent models
- Invalid default values for field types
- Circular dependencies
- Missing required attributes

## Implementation Notes

The parser is implemented as a recursive descent parser in [`parser.rs`](src/parser.rs). Key features:

- **One-token lookahead**: Uses `peek()` to make parsing decisions
- **Error recovery**: Attempts to continue after errors
- **Span tracking**: Every AST node includes source location
- **No left recursion**: Grammar is designed to avoid left recursion
- **No backtracking**: Predictive parsing with single lookahead

## Future Extensions

Grammar features planned for future phases:

- View declarations
- Function declarations
- Trigger definitions
- Advanced constraint syntax
- Custom type definitions
- Native database types (`@db.VarChar(255)`)

---

**Parser Implementation**: [`parser.rs`](src/parser.rs)  
**AST Definitions**: [`ast.rs`](src/ast.rs)  
**Visitor Pattern**: [`visitor.rs`](src/visitor.rs)  
**Lexer Implementation**: [`lexer.rs`](src/lexer.rs)  
**Token Definitions**: [`token.rs`](src/token.rs)
