// Eye-dome lighting: render the scene to a target with depth, then shade each pixel by how
// much nearer its eight neighbours are (log depth). Point colours are sRGB bytes and pass
// straight through both passes untouched, which fixes the washed-out colours the spike had
// (it let three.js convert them twice).
import * as THREE from "three";

export class EdlPass {
  private target: THREE.WebGLRenderTarget;
  private quad: THREE.Mesh<THREE.PlaneGeometry, THREE.ShaderMaterial>;
  private quadScene = new THREE.Scene();
  private quadCamera = new THREE.OrthographicCamera(-1, 1, 1, -1, 0, 1);
  enabled = true;
  strength = 1.0;

  constructor() {
    this.target = new THREE.WebGLRenderTarget(1, 1, { depthTexture: new THREE.DepthTexture(1, 1) });
    this.quad = new THREE.Mesh(
      new THREE.PlaneGeometry(2, 2),
      new THREE.ShaderMaterial({
        uniforms: {
          tColor: { value: this.target.texture },
          tDepth: { value: this.target.depthTexture },
          texel: { value: new THREE.Vector2(1, 1) },
          near: { value: 0.1 },
          far: { value: 1000 },
          strength: { value: 1 },
        },
        vertexShader: `varying vec2 vUv;
          void main() { vUv = uv; gl_Position = vec4(position.xy, 0.0, 1.0); }`,
        fragmentShader: `uniform sampler2D tColor; uniform sampler2D tDepth;
          uniform vec2 texel; uniform float near; uniform float far; uniform float strength;
          varying vec2 vUv;
          float logDepth(vec2 uv) {
            float d = texture2D(tDepth, uv).x;
            if (d >= 1.0) return -1.0;
            float z = d * 2.0 - 1.0;
            return log2(2.0 * near * far / (far + near - z * (far - near)));
          }
          void main() {
            vec4 color = texture2D(tColor, vUv);
            float dc = logDepth(vUv);
            if (dc < 0.0) { gl_FragColor = color; return; }
            float sum = 0.0;
            for (int i = 0; i < 8; i++) {
              float a = float(i) * 0.7853982;
              float dn = logDepth(vUv + vec2(cos(a), sin(a)) * texel * 1.4);
              sum += dn < 0.0 ? 1.0 : max(0.0, dc - dn);
            }
            gl_FragColor = vec4(color.rgb * exp(-sum * 40.0 * strength / 8.0), 1.0);
          }`,
        depthTest: false,
        depthWrite: false,
      }),
    );
    this.quadScene.add(this.quad);
  }

  render(renderer: THREE.WebGLRenderer, scene: THREE.Scene, camera: THREE.PerspectiveCamera) {
    if (!this.enabled) {
      renderer.setRenderTarget(null);
      renderer.render(scene, camera);
      return;
    }
    const size = renderer.getDrawingBufferSize(new THREE.Vector2());
    if (this.target.width !== size.x || this.target.height !== size.y)
      this.target.setSize(size.x, size.y);
    const u = this.quad.material.uniforms;
    u.texel.value.set(1 / size.x, 1 / size.y);
    u.near.value = camera.near;
    u.far.value = camera.far;
    u.strength.value = this.strength;
    renderer.setRenderTarget(this.target);
    renderer.render(scene, camera);
    renderer.setRenderTarget(null);
    renderer.render(this.quadScene, this.quadCamera);
  }

  dispose() {
    this.target.dispose();
    this.quad.geometry.dispose();
    this.quad.material.dispose();
  }
}
