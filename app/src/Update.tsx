// Updates: the program only, from signed release files (the signature is checked against the
// key built into Lotus before anything installs). Nothing is sent but the version check; a
// failed check (offline, no release yet) is quiet.
import { useEffect, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";

export function UpdateCheck({ auto }: { auto?: boolean }) {
  const [update, setUpdate] = useState<Update | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const find = async (quiet: boolean) => {
    if (!quiet) setStatus("Checking…");
    try {
      const u = await check();
      setUpdate(u);
      setStatus(u ? null : quiet ? null : "Lotus is up to date.");
    } catch (e) {
      setStatus(quiet ? null : `Couldn't check for updates (${String(e)}).`);
    }
  };
  useEffect(() => {
    if (!auto) return;
    let live = true;
    check().then(
      (u) => live && setUpdate(u),
      () => {},
    );
    return () => {
      live = false;
    };
  }, [auto]);

  if (update)
    return (
      <p className="update-note">
        Lotus {update.version} is available.{" "}
        <button
          onClick={async () => {
            setStatus("Downloading…");
            try {
              // The installer closes Lotus and starts the new version.
              await update.downloadAndInstall();
            } catch (e) {
              setStatus(`The update didn't install (${String(e)}).`);
            }
          }}
        >
          Install and restart
        </button>
        {status && <span className="muted"> {status}</span>}
      </p>
    );
  if (auto) return null;
  return (
    <p>
      <button onClick={() => void find(false)}>Check for updates</button>
      {status && <span className="muted"> {status}</span>}
    </p>
  );
}
