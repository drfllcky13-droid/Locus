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
}: {
  onClose: () => void;
  /** Opened from a case package: its hash (the SHA-256 of its manifest). */
  packageHash?: string;
}) {
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
        aria-label="About Locus"
        onClick={(e) => e.stopPropagation()}
      >
        <h2>Locus {app?.version}</h2>
        <p className="muted">Forensic scene reconstruction.</p>
        <dl className="facts">
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
          On laptops with two GPUs, Locus asks for the dedicated one. If this shows integrated
          graphics, set Locus to &quot;High performance&quot; in Windows graphics settings.
        </p>
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
          <button onClick={onClose}>Close</button>
        </div>
      </div>
    </div>
  );
}
