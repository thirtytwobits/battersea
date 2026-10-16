# Choose provider features

`battersea-providers` makes transports independently selectable with Cargo features.
Disable defaults to choose the transports your application needs:

```toml
[dependencies]
battersea-providers = { git = "https://github.com/thirtytwobits/battersea", tag = "v{{version}}", default-features = false, features = ["mock"] }
```

The feature definitions from this release's package manifest are:

```toml
{{#include provider-features.toml}}
```

Enabling a transport does not configure credentials or declare a model's capabilities.
The application provides backend configuration and explicit model capabilities.
Unsupported input, output, and structured-output requests fail before provider I/O.
See [provider conversations](contracts/providers.md) and [media jobs](contracts/media-jobs.md).

Use mock transports when developing the independent examples without remote credentials.
