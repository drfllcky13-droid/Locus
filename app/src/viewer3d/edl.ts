// Eye-dome lighting: render the scene to a target with depth, then shade each pixel by how
// much nearer its eight neighbours are (log depth).
//
// Colour: everything is linear inside the pipeline. Point shaders decode their sRGB bytes
// to linear, the target is a linear half-float texture (enough precision in the darks), and
// the composite encodes once for the screen. One conversion each way: the spike, and a first
// attempt here with an sRGB target, both converted twice and washed colours out.
import * as THREE from "three";

export class EdlPass {
  private target: THREE.WebGLRenderTarget;
  private quad: THREE.Mesh<THREE.PlaneGeometry, THREE.ShaderMaterial>;
  private quadScene = new THREE.Scene();
  private quadCamera = new THREE.OrthographicCamera(-1, 1, 1, -1, 0, 1);
  enabled = true;
  strength = 1.0;

  constructor() {
    this.target = new THREE.WebGLRenderTarget(1, 1, {
      depthTexture: new THREE.DepthTexture(1, 1),
      type: THREE.HalfFloatType,
    });
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
          // Written as-is: pixels nothing was drawn on get exactly the page background.
          background: { value: new THREE.Vector3(0x1e / 255, 0x1f / 255, 0x22 / 255) },
        },
        vertexShader: `varying vec2 vUv;
          void main() { vUv = uv; gl_Position = vec4(position.xy, 0.0, 1.0); }`,
        fragmentShader: `uniform sampler2D tColor; uniform sampler2D tDepth;
          uniform vec2 texel; uniform float near; uniform float far; uniform float strength;
          uniform vec3 background;
          varying vec2 vUv;
          // Nothing drawn is tested on the stored depth (the far plane), not on the log
          // depth: that is negative for anything nearer than 1 m, which a sign test once
          // blanked (close-ups of a wall came out empty).
          float logDepth(float d) {
            float z = d * 2.0 - 1.0;
            return log2(2.0 * near * far / (far + near - z * (far - near)));
          }
          void main() {
            vec4 color = texture2D(tColor, vUv);
            float d0 = texture2D(tDepth, vUv).x;
            if (d0 >= 1.0) { gl_FragColor = vec4(background, 1.0); return; }
            float dc = logDepth(d0);
            float sum = 0.0;
            for (int i = 0; i < 8; i++) {
              float a = float(i) * 0.7853982;
              float dn = texture2D(tDepth, vUv + vec2(cos(a), sin(a)) * texel * 1.4).x;
              sum += dn >= 1.0 ? 1.0 : max(0.0, dc - logDepth(dn));
            }
            gl_FragColor = vec4(color.rgb * exp(-sum * 40.0 * strength / 8.0), 1.0);
            #include <colorspace_fragment>
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
