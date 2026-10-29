# Verified parallel snapshot bootstrap

`spec.storage.snapshotRef.backupUrl` now runs the Rust parallel downloader in a
`snapshot-restore` init container on the node's data PVC. Kubernetes waits for
this init to complete before migration init containers and the Stellar process
start. The operator process does not download data into its own filesystem.

Use the configuration in `examples/snapshots/verified-bootstrap.yaml`, replacing
its SHA-256 annotation with the trusted publisher's digest of the compressed
archive. HTTPS on port 443 and S3 `.tar.gz` or `.tar.zst` objects are supported. Archives
must contain regular files and directories relative to the node data root;
links, special files, parent paths, and `.stellar-bootstrap-*` paths are rejected.

Build and publish the operator image containing this change, then set
`snapshotRef.restoreImage` to that image, preferably pinned by digest. It must
contain `/stellar-operator`; AWS CLI and Alpine images cannot run this command.
Alternatively set `STELLAR_SNAPSHOT_RESTORE_IMAGE` on the operator. The fallback
is `ghcr.io/stellar/stellar-k8s:<operator-package-version>` and requires that tag
to contain this command. No additional binary or shell is required in the image.

For S3, `credentialsSecretRef` may contain `AWS_ACCESS_KEY_ID`,
`AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN`, and `AWS_REGION` or
`AWS_DEFAULT_REGION`. Standard AWS SDK credentials also work when available in
the pod. S3 GET requests are presigned inside the init container for 24 hours;
session credentials can expire sooner. For authenticated HTTPS, put
`BEARER_TOKEN` in the referenced Secret. The token is sent as a sensitive
Authorization header and is never placed in the URL or command line.

Restore requires one replica, no autoscaling, and an empty data PVC (filesystem
`lost+found` is allowed). Do not combine it with CSI restore, `restoreFromSnapshot`,
or OCI snapshot pulling. The generated NetworkPolicy allows HTTPS for this
bootstrap; external cluster policies must also permit the configured service.

Downloads use eight workers, 8 MiB ranges, four retries with exponential backoff,
a 300-second network inactivity timeout, and 32 MiB/s per init process by default.
The operator forwards these environment overrides into the generated init:

| Setting | Limit |
| --- | --- |
| `STELLAR_CATCHUP_WORKERS` | 1–64 |
| `STELLAR_CATCHUP_BYTES_PER_SECOND` | positive bytes/second |
| `STELLAR_CATCHUP_RETRIES` | 0–16 |
| `STELLAR_CATCHUP_CHUNK_BYTES` | 1–67,108,864 |
| `STELLAR_CATCHUP_TIMEOUT_SECONDS` | 1–3,600 |
| `STELLAR_SNAPSHOT_MAX_UNPACKED_BYTES` | positive; default 1 TiB |

Extraction also limits archives to 100,000 entries and 128 path components.
Divide bandwidth allocation across concurrent node pods or enforce node-level
traffic shaping; independent init processes cannot share an in-memory limiter.
Reserve enough PVC capacity for the compressed archive and unpacked data.

After SHA-256 verification, extraction finishes in a private directory on the
PVC. A durable manifest records its digest and top-level entries before they
are moved into the data root. A restarted init resumes those moves. The digest
completion marker is published last, so partial data never permits node startup.
An ordinary pod restart skips a matching completed restore. A different digest
or unrelated existing data is rejected; provision a fresh PVC for a new restore.
Keep the `.stellar-bootstrap-*` metadata with the volume. Cross-process file
locking prevents simultaneous restore attempts. Crash durability uses directory
fsync in Linux production containers; local Windows tests use best-effort directory
durability.

Portable backups must exclude root `.stellar-bootstrap-*` entries. The operator's
file-backup collector and OCI tar producer do this automatically. For manual tar
backups, use `tar --exclude='./.stellar-bootstrap-*' -czf snapshot.tar.gz -C DATA .`.
Stop or quiesce the node before taking a database snapshot; the restore marker is
local completion evidence and must never be copied into another node's archive.

Source regression tests cover generated init wiring, missing digest and
conflicting restore sources, partial-publication recovery, pending-marker
recovery, conflicting files, and unsafe source/path rejection. No code, builds,
tests, cluster deployment, or timing benchmarks were executed for this change.
