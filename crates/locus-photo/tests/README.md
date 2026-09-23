# locus-photo benchmark test

`eth3d.rs` checks photogrammetry against a public benchmark whose ground truth comes from a laser scan. It is marked heavy (`#[ignore]`) because it needs two things that are **not in this repository and never ship with the app**:

- **COLMAP**, installed by the user (BSD-3; see `docs/phase8-colmap-licence-review.txt` for its components). Set `LOCUS_COLMAP` to `colmap.exe`. Developed against COLMAP 4.2.0, CUDA build, SHA-256 of the release zip `991e0bae403a496fcc4de0c1f1f428619bf12f8000978f77bc6799d9bfeac23e`.
- **The ETH3D high-resolution multi-view "pipes" scene**: https://www.eth3d.net/datasets, file `pipes_dslr_jpg.7z`, SHA-256 `e2f81386e24dbefcd7090b02431e510b288f61df69591c04fb483056ad0f6ab6`. Extract it so that `pipes/` sits under the folder `LOCUS_ETH3D` names. Windows' own `tar -xf` reads `.7z`.

**Licence of the data:** ETH3D is licensed under Creative Commons Attribution-NonCommercial-ShareAlike 4.0 International (https://creativecommons.org/licenses/by-nc-sa/4.0/). It is used here only as test input, kept outside the repository, and neither redistributed nor bundled.

Source: T. Schöps, J. L. Schönberger, S. Galliani, T. Sattler, K. Schindler, M. Pollefeys and A. Geiger, "A Multi-View Stereo Benchmark with High-Resolution Images and Multi-Camera Videos", CVPR 2017.

Run it from the repository root:

```
LOCUS_COLMAP=…\colmap.exe LOCUS_ETH3D=…\eth3d cargo test --release -p locus-photo --test eth3d -- --ignored --nocapture
```

Options:
- `LOCUS_COLMAP_GPU=1` uses COLMAP's GPU SIFT (SiftGPU, which has non-commercial terms). By default the test extracts features on the CPU.
- `LOCUS_COLMAP_SIZE` sets the image size used for feature extraction (default 4800 px).
- `LOCUS_COLMAP_MODEL` sets the camera model (default OPENCV_FISHEYE).

**What it checks.**
1. Every reconstructed point is triangulated again from its own image observations, this time with the ground-truth cameras. This gives its true position.
2. The model is scaled by three or four known distances of 2–3 m between well-triangulated points.
3. Every other distance of at least 0.5 m is compared with the truth.
4. The bound: 99 % of them within 1 %.

**Result on 2026-09-23** (COLMAP 4.2.0, CPU features, 4800 px):
- 14 of 14 images registered;
- 94,384 check distances: median error 0.16 %, 99th percentile 0.58 %, worst 1.01 %;
- 100.00 % within 1 % to two decimals: the worst, 1.011 %, is the one exception.
