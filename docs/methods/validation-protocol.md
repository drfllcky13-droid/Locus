# Physical validation protocol

The validation report (`locus-validate run`, [validation.md](validation.md)) shows that each tool recovers known truth on synthetic data. That is necessary but not enough for court. Synthetic data can't model real surfaces, lighting, scanner behaviour, or how examiners actually pick points and read stains. This protocol describes the physical studies that measure Locus's accuracy the way it will be used: on staged scenes with measured truth, by several examiners working blind.

It follows the general design of the published black-box and white-box studies in forensic science: known ground truth, independent examiners, blinding, pre-registered analysis, and error rates reported with their confidence intervals.

## 1. Roles

- **Study lead**: designs each scene, records its ground truth, holds the answer key, and analyses the results. Never an examiner in the same study.
- **Examiners**: at least **five** per study, of mixed experience, trained on Locus to the same standard (the guided workflow plus the method notes). They see only what an examiner sees at a real scene.
- **Independent reviewer**: checks the ground-truth measurements and the analysis before results are released.

## 2. Ground truth

Each scene's truth is measured with an instrument at least **five times more precise** than the error being studied, and that instrument's own calibration is recorded:

| Tool | Truth measured with | Target precision of the truth |
|---|---|---|
| Measurements, registration | A total station or a calibrated tape on surveyed control; scale bars with calibration certificates | ≤ 1 mm |
| Bullet trajectory | Fired through set-up panels at a surveyed muzzle position, or rods through drilled holes at surveyed angles | ≤ 0.1° |
| Bloodstain area of origin | Blood (or a validated substitute) projected from a surveyed source point, with the source's position recorded for every event | ≤ 1 cm |
| Camera matching and height | People of measured stature (without footwear, and with it, recorded), standing at surveyed marks; the camera surveyed | ≤ 2 mm (stature), ≤ 5 mm (camera) |
| Crash (skid, yaw, momentum, crush) | Instrumented test drives (a calibrated GNSS/IMU speed log) on a measured surface, and a surveyed crush profile | ≤ 0.5 km/h; ≤ 5 mm |
| Photogrammetry | Surveyed targets, and check distances not used for scaling | ≤ 1 mm |

## 3. Design

- **Scenes**: at least **three** per tool, spanning easy, typical and hard conditions (for example good light and square-on photos; a cluttered room; low light and oblique photos). Each is documented with photos and a written description.
- **Blinding**: examiners get the scene data (scans, photos, notes an attending officer would have) and nothing else. They don't see the truth, each other's work, or which scenes are "hard".
- **Independence**: each examiner processes every scene alone, in a random order, recording time taken.
- **Replication**: each examiner repeats at least one scene at least **two weeks** later without seeing their first result, which gives within-examiner repeatability.
- **Pre-registration**: before any data is collected, write down:
  - the bounds each tool is expected to meet (the synthetic validation's, widened by the truth's uncertainty);
  - how outliers will be handled (never silently dropped; reported with the reason);
  - how every result will be analysed.

## 4. What each examiner records

Examiners use Locus as they would in casework, and each submits:
- the project (the `.locus` folder), so every pick and setting is in its audit log;
- each result with its stated uncertainty, as Locus reports it;
- any judgment calls (an excluded stain, a chosen lens model, an assumed segment), with the reason;
- time taken, and any problems.

## 5. Analysis

For each tool and scene:
- **Error**: the result minus the truth, per examiner and scene. Report the mean, standard deviation, 95th percentile and worst, with the truth's own uncertainty stated alongside.
- **Coverage**: the share of results whose stated 95 % interval or region contains the truth, with its binomial 95 % confidence interval. The target is about 95 %; well below that means the stated uncertainty is too small.
- **Between-examiner reproducibility**: the spread of results across examiners on the same scene.
- **Within-examiner repeatability**: the difference between an examiner's first and repeated results.
- **Conventional method**: where Locus reports it (the bloodstain conventional point, for example), the same statistics for it, so the choice of primary method stays justified on real data.
- **Failures**: every result that misses its bound, with its cause (tool, examiner, scene condition). Where a failure shows a software problem, it is logged and fixed. The study is then rerun on the fixed version, not re-scored.

## 6. Reporting

- A study report gives the scenes, truth and its uncertainty, examiners (anonymised), the pre-registered plan, all results (none withheld), the statistics above, the failures and their causes, and the Locus version and validation report it used.
- The report is kept with the release it validates. The validation report's "What this does and doesn't show" section points to it.
- A new release that changes a tool's method needs that tool's study repeated, at least on the typical scene.

## 7. Open

Who runs the studies, and when, is Addison's decision (docs/ADDISON-TODO.md).
