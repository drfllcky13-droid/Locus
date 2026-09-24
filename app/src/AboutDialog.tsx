import { open as openDialog } from "@tauri-apps/plugin-dialog";
import type { LicenseInfo } from "./license";
import { useEffect, useState } from "react";
import { api } from "./api";

interface GpuInfo {
  webgl: string;
  version: string;
  webgpu: string;
}

/** The GPU WebGL actually renders with: WebView2 picks it, not locus.exe. */
function webglGpu(): Pick<GpuInfo, "webgl" | "version"> {
  const gl = document
    .createElement("canvas")
    .getContext("webgl2", { powerPreference: "high-performance" });
  if (!gl) return { webgl: "WebGL 2 unavailable", version: "" };
  const dbg = gl.getExtension("WEBGL_debug_renderer_info");
  return {
    webgl: String(
      dbg ? gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER),
    ),
    version: String(gl.getParameter(gl.VERSION)),
  };
}

interface GpuAdapter {
  info?: { vendor?: string; architecture?: string; description?: string };
}

async function webgpu(): Promise<string> {
  const gpu = (
    navigator as unknown as { gpu?: { requestAdapter(o: object): Promise<GpuAdapter | null> } }
  ).gpu;
  if (!gpu) return "not available";
  const a = await gpu.requestAdapter({ powerPreference: "high-performance" });
  if (!a) return "no adapter";
  const i = a.info ?? {};
  return ["available", i.vendor, i.architecture, i.description].filter(Boolean).join(", ");
}

export function AboutDialog({
  onClose,
  packageHash,
  license,
  onLicense,
}: {
  onClose: () => void;
  /** Opened from a case package: its hash (the SHA-256 of its manifest). */
  packageHash?: string;
  license?: LicenseInfo | null;
  onLicense?: (l: LicenseInfo) => void;
}) {
  const [licenseError, setLicenseError] = useState<string | null>(null);
  const [app, setApp] = useState<{ version: string; webview: string } | null>(null);
  const [gpu] = useState(webglGpu);
  const [gpuWeb, setGpuWeb] = useState("checking…");
  const [notices, setNotices] = useState<string | null>(null);
  useEffect(() => {
    void api.appInfo().then(setApp);
    void webgpu().then(setGpuWeb);
  }, []);
  return (
    <div className="overlay" onClick={onClose}>
      <div
        className="dialog"
        role="dialog"
        aria-label="About Lotus"
        onClick={(e) => e.stopPropagation()}
      >
        <h2>Lotus {app?.version}</h2>
        <p className="muted">Forensic scene reconstruction.</p>
        <dl className="facts">
          <dt>Licence</dt>
          <dd>
            {license?.license
              ? `${license.tier_name}, ${license.license.licensee} (${license.license.id}${license.license.expires ? `, until ${license.license.expires}` : ""})`
              : (license?.tier_name ?? "…")}
          </dd>
          {packageHash && (
            <>
              <dt>Case package hash</dt>
              <dd className="hash">{packageHash}</dd>
            </>
          )}
          <dt>Rendering GPU</dt>
          <dd>{gpu.webgl}</dd>
          <dt>WebGL</dt>
          <dd>{gpu.version}</dd>
          <dt>WebGPU</dt>
          <dd>{gpuWeb}</dd>
          <dt>WebView2</dt>
          <dd>{app?.webview}</dd>
        </dl>
        <p className="muted">
          On laptops with two GPUs, Lotus asks for the dedicated one. If this shows integrated
          graphics, set Lotus to &quot;High performance&quot; in Windows graphics settings.
        </p>
        {licenseError && <p className="error">{licenseError}</p>}
        {notices !== null && (
          <pre className="notices" aria-label="Third-party notices">
            {notices}
          </pre>
        )}
        <div className="buttons">
          <button
            onClick={() =>
              notices === null ? void api.thirdPartyNotices().then(setNotices) : setNotices(null)
            }
          >
            {notices === null ? "Third-party notices" : "Hide notices"}
          </button>
          {onLicense && (
            <button
              onClick={async () => {
                const path = await openDialog({
                  title: "Install a licence",
                  filters: [{ name: "Lotus licence", extensions: ["locus-license"] }],
                });
                if (typeof path !== "string") return;
                try {
                  onLicense(await api.licenseInstall(path));
                  setLicenseError(null);
                } catch (e) {
                  setLicenseError(String(e));
                }
              }}
            >
              Install a licence…
            </button>
          )}
          <button onClick={onClose}>Close</button>
        </div>
      </div>
    </div>
  );
}
