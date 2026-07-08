Settings
========

`lord` can be configured with the command line, environment variables, a
configuration file, and default values.

The command line takes precedence over environment variables, which take
precedence over the configuration file, which takes precedence over defaults.

The path to the configuration file can be given with `--config <CONFIG_PATH>`.
`lord` will error if `<CONFIG_PATH>` doesn't exist.

When no explicit `--config` is set, Lord probes **exactly one directory** for
config files. Directory selection precedence:

1. `--config-dir <CONFIG_DIR_PATH>` if set
2. else `--datadir <DATA_DIR_PATH>` if set
3. else the default data directory

Within that single directory, Lord probes files in this order:

1. `lord.yaml`
2. `ord.yaml` (ord compatibility)

It is not an error if neither file exists in the probed directory.

If both `--config-dir` and `--datadir` are passed, **`--config-dir` wins** for
config probing; `--datadir` still sets Lord's data directory for index, calendar,
and storage paths.

For a setting named `--setting-name` on the command line, the environment
variable will be named `ORD_SETTING_NAME`, and the config file field will be
named `setting_name`. For example, the data directory can be configured with
`--datadir` on the command line, the `ORD_DATA_DIR` environment variable, or
`data_dir` in the config file.

See `lord --help` for documentation of all the settings.

`lord`'s current configuration can be viewed as JSON with the `lord settings`
command.

Example Configuration
---------------------

New deployments should use `lord.yaml`:

```yaml
{{#include ../../../lord.yaml}}
```

`ord.yaml` remains valid for ord compatibility:

```yaml
{{#include ../../../ord.yaml}}
```