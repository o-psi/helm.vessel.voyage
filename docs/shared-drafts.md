# Shared drafts removed

Shared composer drafts and all unsent-content persistence have been removed.
Web and native Helm retain unsent text and attachments only in client memory.
Closing or reloading the client loses that composition. There is no draft
polling, discovery, cross-device synchronization, or server-side draft staging.

Pictures are uploaded directly to the destination Voyage before multimodal
submission. Durable execution identities and receipts remain; these are not
draft storage. Existing saved draft files are left untouched but are no longer
loaded or updated.
