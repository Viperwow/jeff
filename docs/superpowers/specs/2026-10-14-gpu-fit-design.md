# Several local models on one GPU

## Why

jeff installs local models (CLM, H2O-Lightning) in Docker. A user with enough GPU memory should be able to run more than one at a time. A user without enough should learn that before the install starts, and see how much is missing, instead of an install that reports success and then a model that never loads.

PR #17 refuses any install while another local model runs. This design replaces it.

## Goals

- Before an install starts, show how much GPU memory it will take and whether it fits beside what already runs.
- Let models that fit run at the same time.
- When the model does not fit, offer one action that stops the other local models and installs this one.
- Give the CLI the same check and the same way out.

## Non-goals

- Splitting one model across several GPUs, or choosing which GPU a model uses. jeff reads the first GPU only.
- Freeing memory held by other programs.
- AMD, Intel or Apple GPUs.

## Anti-goals

- An install that starts while jeff already knows it will not fit, without the user choosing to stop the others.
- A "fits" verdict that turns out wrong because the models were started with vLLM's default memory share: each vLLM reserves a fixed share of the whole GPU, so two at 0.85 never fit.

## The confirmation, as approved

The final mockup is `gpu-fit-v19.html` from the brainstorming session. It sits on the provider card of a local model, in the install block.

1. The card shows Install and Uninstall as today.
2. Clicking Install does not start anything. Install and Uninstall are replaced by two buttons:
   - `Install` (primary) when the model fits, or `Stop current and install` (danger) when it does not. The button never names a model.
   - `Cancel`, which restores Install and Uninstall.
3. Below the buttons, a GPU memory bar, 5 px tall:
   - Above it, on the left: `Required GPU memory X GB`, the total after the install.
   - A white mark on the bar at the GPU's size, labelled `Your GPU N GB`. When the model fits, the bar's scale is the GPU size, so the mark sits at the right edge and the free part is the empty track. When it does not fit, the scale is the required total plus about 8%, fixed when the confirmation opens, so the mark stays put while the live reading changes.
   - Segments, left to right: other programs (light grey, solid), each running jeff model (its own solid colour), then the new model, hatched. Hatching means "what the new model needs". Colour means whether that part fits: green up to the GPU mark, red after it. When the model does not fit, the red part always reaches the right edge.
4. Below the bar, a legend in blocks separated by a vertical line:
   `Other apps 0.7 GB, live | H2O-Lightning 11 GB | CLM requires 9.5 GB: 4.3 GB fits, 5.2 GB does not fit`.
   When it fits, the last block is `CLM requires 9.5 GB, all fits` followed by `Free X GB`.
5. While the confirmation is open, the reading of other programs refreshes about every 2 seconds. Only the grey segment and the numbers change; the scale and the mark do not.
6. Confirming starts the install, and the card shows the usual install progress.

When jeff cannot read the GPU (no NVIDIA card, no `nvidia-smi`, or no answer from it), the confirmation follows `gpu-unknown-v4.html`, variant U2:

- The buttons are `Install anyway` (primary) and `Cancel`.
- The bar's scale is the required total: jeff's running models plus the new one, each segment proportional to its size. There is no other-programs segment and no GPU mark. The new model's segment is hatched in neutral grey, not green or red.
- Above the bar, on the left: `Required GPU memory X GB`; on the right: `Your GPU: unknown`.
- The legend lists the running models and `CLM requires 9.5 GB`, with the same separators.
- Below, in orange: `Could not read your GPU memory, so it cannot tell whether this fits.`

## Behaviour, as decided

- Models that fit run at the same time.
- `Stop current and install` stops every other local model jeff runs, the same way Uninstall does: the containers go, the downloaded weights stay. Then it installs this model.
- The CLI gets the same check. `jeff install <stack>` refuses when the model does not fit, prints the same numbers, and suggests `--stop-others`. `jeff install <stack> --stop-others` stops the others and installs.
- The memory a model needs comes from its description in jeff. The GPU size and the memory in use come from `nvidia-smi`.
- A model is installed into the free space: it always starts with its own share (required GB ÷ GPU GB), even when it runs alone. Installing one never restarts or resizes a model that already runs. The cost, accepted: a model alone gets less memory for concurrent requests than today's 0.85.

## Constraints

- `nvidia-smi` on Windows reports `[N/A]` for the memory of each process, so jeff cannot ask which process holds what. Other programs = memory in use − what jeff's running models need, floored at 0.
- vLLM reserves `--gpu-memory-utilization` × the whole GPU at start and refuses to start if that much is not free. To run side by side, each stack must start with its own share: required GB ÷ GPU GB. jeff writes that share into the compose file it renders at install time, instead of the fixed 0.85.
- H2O-Lightning also needs its Mamba state cache to hold `--max-num-seqs` sequences (64 today), or vLLM refuses to start. Its required memory must leave room for that.
- Existing installs keep working: a stack started before this change still runs at 0.85 until it is reinstalled. jeff counts a running stack at its declared requirement, so the bar may understate it; a reinstall fixes that.
- `install::run` already refuses a port taken by another program. That check stays.

## What exists to build on

- `src/install.rs`: `Stack` (`Clm`, `H2oLightning`) with `id`, `name`, `port`, `running()`, `compose()` and `steps()`; `run(stack, action, ...)` renders `~/.jeff/<id>/compose.yaml` and runs `docker compose`.
- `deploy/clm/compose.yaml` and `deploy/h2o-lightning/compose.yaml`, both with `--gpu-memory-utilization 0.85`.
- API: `POST /api/{stack}/install|remove`, `GET /api/{stack}` for progress. The admin page calls these.
- UI: `ui/app.js` `providerCard` and `installTask`; the install block in `ui/index.html` (`.p-install`, `.p-install-btn`, `.p-remove-btn`).

## Left to the builder

- The exact required GB per stack. Measure each on the RTX 5070 Ti (16 GB): the smallest share at which the model loads and serves its default batch, plus a margin. The mockup used 11 GB for H2O-Lightning and 9.5 GB for CLM; those are guesses.
- The shape of the API that serves the numbers to the page and the CLI, and the parameter that asks an install to stop the others. Following the existing pattern, `GET /api/{stack}/fit` and `POST /api/{stack}/install` with `{"stop_others": true}` would fit, but the builder decides.
- Colours for running models beyond H2O-Lightning's sky blue, taken from the page's existing palette.
- Tests: the fit arithmetic and the share written into the compose file need unit tests; reading `nvidia-smi` needs a parser test on captured output.

## Calls I made (not discussed)

- jeff reads only the first GPU (`nvidia-smi` index 0).
- The fit check counts other programs as measured at the moment of the check; it does not reserve headroom for them beyond the margin in each model's requirement.
- The confirmation refresh interval is 2 seconds.
- PR #17 is closed without merging once this ships.
- Reinstalling a model that runs does not count it twice: the check leaves the model being installed out of the running set.
- When jeff cannot read the GPU, the CLI installs without a check, as it does today.
