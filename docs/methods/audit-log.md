# Audit log and evidence integrity

## What is recorded

Every change to a project's analysis state writes one entry to the `audit_log` table in `project.sqlite`, in the same database transaction as the change. The change and its entry are either both saved or both not saved. Opening a project is also logged, with the examiner name given at open.

Each entry holds:

| Field | Meaning |
|---|---|
| `seq` | 1, 2, 3, … with no gaps |
| `timestamp` | UTC, RFC 3339, millisecond precision, from the workstation clock |
| `actor` | examiner name entered when the project was created or opened |
| `action` | e.g. `project.created`, `project.opened`, `evidence.imported`, `evidence.verified` |
| `details` | JSON object describing the change (for imports: SHA-256, size, original path, unit, counts, warnings) |
| `prev_hash` | `hash` of the entry before it; 64 zeros for the first entry |
| `hash` | SHA-256 over the fields above |

## How the hash is computed

`hash = SHA-256( len‖prev_hash ‖ len‖seq ‖ len‖timestamp ‖ len‖actor ‖ len‖action ‖ len‖details )`

Each field is preceded by its length as an unsigned 64-bit little-endian integer, and `seq` is hashed as a signed 64-bit little-endian integer. Length prefixes mean two different entries can't produce the same byte stream by shifting text between fields. `details` is hashed exactly as stored, so no JSON re-serialisation is involved.

## What is checked on open

Locus refuses to open a project unless all of these hold:

1. Entries are numbered 1…N with no gaps or duplicates.
2. Each entry's `prev_hash` equals the previous entry's `hash`.
3. Each entry's `hash` recomputes correctly from its stored fields.
4. The last entry's `seq` and `hash` equal the chain head recorded separately in the `meta` table.

Rules 1–3 detect any edited, deleted, inserted or reordered entry. Rule 4 detects entries removed from the end and entries appended outside Locus. The error names the first entry that fails.

The database also has triggers that reject `UPDATE` and `DELETE` on the audit log and on evidence records, so no code path in the application can rewrite them.

## Evidence files

On import, the source file is opened read-only and streamed through SHA-256 while it is copied into `evidence/`. The copy is then hashed again from disk. The import is refused if either hash differs from the hash shown to the examiner at preview. The stored copy is marked read-only. **Verify evidence** re-hashes every stored file, reports intact, missing or changed for each one, and logs the result.

## Limitations

- **The log is self-contained.** Someone with write access to `project.sqlite`, who understands this scheme, could rebuild the whole chain and the recorded head consistently. The chain proves internal consistency, not authorship. To anchor it externally, record the chain head hash outside the project, for example in the case file, a signed report, or an evidence-management system. Reports (Phase 10) will print the head hash for this purpose.
- **Timestamps come from the workstation clock** and are only as accurate as that clock.
- **The actor is a typed name, not an authenticated identity.**
- **Read-only marking** of evidence copies prevents accidental changes, not deliberate ones. Deliberate changes are caught by **Verify evidence**, not prevented.
