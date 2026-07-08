Moderation
==========

Inscription moderation was **removed** with the slim Lord server (PR1a).
Lord no longer serves inscription content and does not support hiding
inscription IDs via config.

The `hidden:` YAML key and `ORD_HIDDEN` environment variable are **not**
supported. `Settings` uses `deny_unknown_fields`, so adding `hidden:` to
`lord.yaml` or `ord.yaml` will cause a deserialize error.

If inscription moderation returns in a future release, it will be re-added to
`Settings` and `lord server` before being documented here. For current operator
configuration, see [Settings](settings.md).