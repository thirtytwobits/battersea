# Media jobs

Status: implemented for v0.9.0.

Synchronous providers return `MediaSubmission::Complete`. Runway and Replicate return a job
with one accepted provider identity. `snapshot` inspects retained state without I/O; `poll`
performs one status request. Queued/running jobs may become succeeded, failed, cancelled or
expired. Unknown statuses and mismatched identities fail. Terminal updates are idempotent.

`wait` retries only status reads, using bounded exponential delays and Retry-After. `generate`
applies one deadline across submission, polling and retrieval. A failed wait cancels active
remote work within a separate five-second cleanup window. Dropping an active job schedules the
same bounded cleanup while the Tokio runtime remains alive. Cleanup failure preserves the job
identity and reports uncertainty. Applications needing crash recovery must retain their own
accepted-job records.

`retrieve` downloads every output before returning a result, retaining it for subsequent calls.
A failed retrieval can be repeated on the same job. It never submits generation. Outputs have
an aggregate encoded-byte bound, optionally reduced with `maxOutputBytes`. Missing/expired
outputs, mismatched MIME types and truncated downloads fail without partial success. Provider
credentials go only to the configured API origin or Replicate's HTTPS delivery domains.
Redirects re-evaluate credential eligibility.

The snapshot exposes output expiry and whether its timestamp is an estimate. Runway uses the
lower end of its documented 24–48 hour output lifetime; Replicate uses completion plus its
documented one-hour default retention. Unknown completion time leaves expiry unknown.
Retrieved bytes remain usable after the remote output expires. Applications own persistent
storage, claims and promotion.

Replicate supports image, video and audio model outputs. `model` names `owner/name` or
`owner/name:version`. `options.inputFields` maps `prompt` and the selected generic parameters to
that model's actual input keys; `options.input` supplies other model settings. Conflicting mappings
fail before dispatch. `outputPointer` optionally selects the media URL or URL array within the
prediction's output. Model-specific capability declarations remain the application's responsibility.

An optional HTTPS `webhookUrl` requests completed callbacks. The host routes raw callback bytes
and passes its signing secret to the accepted job's `webhook` operation. Verification covers
HMAC-SHA256, delivery ID, timestamp (five-minute tolerance), body bytes and prediction identity.
Duplicates and late callbacks cannot replace a terminal outcome or repeat retrieval. HTTP routing,
secret loading and storage are host responsibilities. Polling remains available on the same job.

Protocol sources: [Runway outputs](https://docs.dev.runwayml.com/assets/outputs/),
[Replicate API](https://replicate.com/docs/reference/http),
[callback setup](https://replicate.com/docs/topics/webhooks/setup-webhook),
[callback verification](https://replicate.com/docs/topics/webhooks/verify-webhook).
