/**
 * jeff-skill-suggestion — Claude Code mod
 *
 * Takes the skill listing out of the context window and has jeff, the
 * user's own typed-decision server, suggest at most one skill per prompt,
 * going by the skills' descriptions. The skills stay installed and loadable;
 * what goes away is the listing the engine sends the model every session,
 * one line per skill, whether the prompt has anything to do with any of them.
 *
 * Two requests per prompt, both to jeff's `/v1/systemone`: one ranks every
 * skill and asks jeff's `skill-gate` classifier whether the prompt needs a
 * skill at all, one re-reads the top few with their full text and lets each
 * be rejected on its own. Either may come back empty-handed.
 *
 * The gate's questions live in jeff, not here: the mod reads which questions
 * `skill-gate` holds and asks them by key, so they can be reworded, added or
 * moved to another model on jeff's admin page. When jeff has no `skill-gate`,
 * the mod saves one; a jeff with no access keys takes that without a key,
 * and one with keys needs an admin key in `JEFF_API_KEY`.
 *
 * Hooks:
 *   prompt.attachment  — the engine's `skill_listing` attachment is answered
 *                        with `{ text: null }` (left out) or trimmed to the
 *                        names in `alwaysListed`. Its names are remembered:
 *                        they are the engine's word on which skills the model
 *                        may invoke.
 *   prompt.submit      — the two requests run, and the winner (if any) is
 *                        attached to the prompt as a `<skill_relevance>`
 *                        block: with the skill's own SKILL.md inside it
 *                        (`inject: "content"`, the default), so the skill
 *                        loads even when `skillOverrides` hides it from the
 *                        model, or with its name for the Skill tool
 *                        (`inject: "suggest"`).
 *   skill.prompt       — observation: whether the model took the suggestion,
 *                        or loaded a skill on its own. Also writes the prompt
 *                        of the plugin's own `/jeff-skill-suggestion:setup`,
 *                        which hides every skill from the engine's listing
 *                        (user-invocable-only) once the person has seen the
 *                        list and said yes; the model makes the edit with its
 *                        own tools, so it shows and asks like any other.
 *
 * The candidates come from `$.command.list()`, not from the listing: the
 * listing is only rendered at the turn's first request, after `prompt.submit`
 * has run, so the first prompt of a session would otherwise have nothing to
 * choose from. The listing, once seen, narrows the candidates to what the
 * engine itself would have shown.
 *
 * Only the main conversation is handled. A subagent's own listing is left as
 * the engine renders it: its prompt is a tool call's argument, not a
 * `prompt.submit`, so nothing here could suggest for it.
 *
 * Every failure path is fail-open: jeff not running, a request that errors or
 * runs past the latency budget lets the prompt through with no suggestion.
 *
 * Privacy: the prompt text, every candidate skill's name and description, and
 * the opening of each shortlisted skill's SKILL.md are sent to jeff, and from
 * there to whichever model jeff routes the questions to.
 */
import type { Register } from 'claude-code'
import {
  DEFAULT_ADMIN_URL,
  DEFAULT_URL,
  GATE_CLASSIFIER,
  GATE_QUESTIONS,
  SETUP_COMMAND,
  baseUrl,
  canonical,
  catalog,
  commandLike,
  decide,
  detailOf,
  displayIds,
  gateClassifier,
  injectionBlock,
  installPathsOf,
  keySafe,
  modelInvocable,
  parseListing,
  parseNames,
  pluginPrefix,
  passesGate,
  pluginFileCandidates,
  readGate,
  readRerank,
  readSkillSettings,
  readWide,
  requestBody,
  requestHeaders,
  rerankQuestions,
  setupAborted,
  setupInstructions,
  setupPlan,
  shortlistOf,
  skillFileCandidates,
  summaryOf,
  suggestionBlock,
  syncedFileCandidates,
  trimListing,
  validBackup,
  wideQuestions,
} from './policy.ts'
import type { Candidate, Gate, PolicyConfig, Rerank, Skill, Wide } from './policy.ts'

/** Prompt origins that are not a task of the person's: nothing to suggest for. */
const NOT_A_TASK = new Set([
  'task-notification',
  'peer',
  'peer-send-message',
  'projects-relay',
  'observer',
  'observer-activity',
])

export const register: Register = (on, options) => {
  const text = (key: string, fallback: string) =>
    typeof options[key] === 'string' && options[key] ? (options[key] as string) : fallback
  const number = (key: string, fallback: number) =>
    typeof options[key] === 'number' ? (options[key] as number) : fallback
  const flag = (key: string, fallback: boolean) =>
    typeof options[key] === 'boolean' ? (options[key] as boolean) : fallback

  const hideListing = flag('hideListing', true)
  // "content": the mod reads the chosen skill's SKILL.md and attaches it, so
  // the skill loads even when the engine lists it as user-invocable-only or
  // off. "suggest": the block alone, and the model loads the skill with the
  // Skill tool, which honours the engine's skillOverrides.
  const injectContent = text('inject', 'content') !== 'suggest'
  const alwaysListed = parseNames(text('alwaysListed', ''))
  const neverSuggested = parseNames(text('neverSuggested', ''))
  const rerankEnabled = flag('rerank', true)
  const excerptChars = number('excerptChars', 700)
  const timeoutMs = number('timeoutMs', 3000)
  const logDecisions = flag('logDecisions', true)
  const policy: PolicyConfig = {
    shortlist: Math.max(1, Math.round(number('shortlist', 3))),
    gateThreshold: number('gateThreshold', 0.2),
    fitsThreshold: number('fitsThreshold', 0.3),
  }

  // The names every skill_listing attachment carried so far. Once non-empty,
  // only these are offered to jeff: the listing is the engine's word on which
  // skills the model is allowed to invoke, and `$.command.list()` also names
  // commands the model may not.
  const listed = new Set<string>()
  // Each skill's SKILL.md as first found, with where, or null when nowhere:
  // read once per session, since the second request wants it on every prompt
  // it is on, and the injection wants it whole.
  const files = new Map<string, { path: string; markdown: string } | null>()
  // The skills whose instructions were already attached this session: a
  // second time, the block only names the skill again.
  const injected = new Set<string>()
  // The same failure on every prompt would bury the log; each kind is said once.
  const reported = new Set<string>()

  // A skill whose frontmatter `name:` has spaces ("PocketBase API Rules") is
  // reported by `$.command.list()` under that name, but the engine lists,
  // runs and overrides it by its directory name (`pb-api-rules`). The map
  // from one to the other is read from disk once per session, in whichever
  // hook first needs it.
  let displayToId: Map<string, string> | null = null

  // Options win over the environment, which is where the jeff CLI reads them.
  let jeff: { url: string; adminUrl: string; apiKey: string } | null = null
  // `skill-gate` as jeff holds it, read once per session once it answers.
  let gate: Gate | null = null

  on('prompt.attachment', { type: 'skill_listing' }, async ($, e, next) => {
    const skills = parseListing(e.text)
    for (const skill of skills) listed.add(skill.name)

    // A subagent's listing is not ours: nothing here suggests for a subagent,
    // so hiding its listing would leave it with no skills at all.
    if (!hideListing || e.agentId) return next(e)

    // Answered without `next`: the engine's text never reaches the model.
    return { text: trimListing(e.text, alwaysListed) }
  })

  on('prompt.submit', async ($, e, next) => {
    if (!jeff) {
      jeff = {
        url: baseUrl(text('url', (await $.env.get('JEFF_URL')) || DEFAULT_URL)),
        adminUrl: baseUrl(text('adminUrl', (await $.env.get('JEFF_ADMIN_URL')) || DEFAULT_ADMIN_URL)),
        apiKey: text('apiKey', (await $.env.get('JEFF_API_KEY')) ?? ''),
      }
    }
    const { url, adminUrl, apiKey } = jeff

    // Notifications and peer messages are not tasks; a typed `/name` already
    // names its skill. Neither gets a suggestion.
    if (!e.text.trim() || /^\/\S/.test(e.text.trim())) return next(e)
    if (e.origin && NOT_A_TASK.has(e.origin.kind)) return next(e)

    const once = (kind: string, line: string) => {
      if (reported.has(kind)) return
      reported.add(kind)
      $.ui.log(`${line}`)
    }

    /** The key for `target`, or none when it would cross the network in plain text. */
    const sendKey = (target: string) => {
      if (!apiKey || keySafe(target)) return apiKey
      once('insecure', `not sending JEFF_API_KEY to ${new URL(target).origin}: use https, or http on localhost`)
      return ''
    }

    /** One call to jeff, or null on timeout or a network error. */
    const call = async (target: string, method: 'GET' | 'POST', body?: string) => {
      try {
        const response = await Promise.race([
          $.http.fetch(target, { method, headers: requestHeaders(sendKey(target)), ...(body ? { body } : {}) }),
          $.clock.sleep(timeoutMs),
        ])
        if (!response) once('timeout', `jeff did not answer ${target} within ${timeoutMs}ms; no suggestion`)
        return response ?? null
      } catch (error) {
        once('unreachable', `jeff is not reachable at ${url} (${String(error)}); start it with \`jeff\`, no suggestion until then`)
        return null
      }
    }

    /** One `/v1/systemone` request, or null on any failure. */
    const ask = async (questions: Record<string, unknown>, what: string, model: string | null): Promise<string | null> => {
      const response = await call(`${url}/v1/systemone`, 'POST', requestBody(e.text, questions, model))
      if (response && response.ok) return response.text
      if (response) once(`${what}-${response.status}`, `jeff responded ${response.status} to the ${what}: ${response.text.slice(0, 200)}`)
      return null
    }

    /**
     * Saves `skill-gate` in jeff: each question on its own, so one the user
     * already has under the same key is kept as it is, then the classifier.
     * Through the admin port, which a jeff with no keys opens to anyone local.
     */
    const saveGate = async (): Promise<boolean> => {
      for (const [key, question] of Object.entries(GATE_QUESTIONS)) {
        const saved = await call(`${adminUrl}/api/questions`, 'POST', JSON.stringify({ [key]: question }))
        if (!saved || (!saved.ok && saved.status !== 409)) {
          if (saved) once('save', `could not save question ${key} in jeff (${saved.status}): ${saved.text.slice(0, 200)}${saved.status === 401 || saved.status === 403 ? '; set JEFF_API_KEY to an admin key' : ''}`)
          return false
        }
      }
      const saved = await call(`${adminUrl}/api/classifiers`, 'POST', JSON.stringify(gateClassifier()))
      if (!saved || (!saved.ok && saved.status !== 409)) {
        if (saved) once('save', `could not save classifier ${GATE_CLASSIFIER} in jeff (${saved.status}): ${saved.text.slice(0, 200)}`)
        return false
      }
      $.ui.log(`saved classifier ${GATE_CLASSIFIER} in jeff with ${Object.keys(GATE_QUESTIONS).join(', ')}`)
      return true
    }

    /** `skill-gate` from jeff, saved first when jeff has none; null while jeff cannot give it. */
    const loadGate = async (): Promise<Gate | null> => {
      if (gate) return gate
      const target = `${url}/v1/classifiers/${GATE_CLASSIFIER}`
      let response = await call(target, 'GET')
      if (response && response.status === 404 && (await saveGate())) response = await call(target, 'GET')
      if (!response) return null
      if (!response.ok) {
        once('gate', `jeff responded ${response.status} for classifier ${GATE_CLASSIFIER}; no suggestion`)
        return null
      }
      gate = readGate(response.text)
      if (!gate) once('gate', `jeff's ${GATE_CLASSIFIER} is not a classifier with questions; no suggestion`)
      return gate
    }

    /** A skill's file, found on disk by Claude Code's layout, or null. */
    const fileOf = async (
      skill: Skill,
      plugin: string | undefined,
    ): Promise<{ path: string; markdown: string } | null> => {
      const cached = files.get(skill.name)
      if (cached !== undefined) return cached
      let found: { path: string; markdown: string } | null = null
      try {
        const home = (await $.env.get('HOME')) ?? ''
        const relative = skillFileCandidates(skill.name, plugin)
        // The engine reads the project's `.claude/` (the working directory
        // only, not its ancestors) and the user's.
        const candidates = [...relative, ...(home ? relative.map((file) => `${home}/${file}`) : [])]
        if (plugin && home) {
          const installed = `${home}/.claude/plugins/installed_plugins.json`
          if (await $.fs.exists(installed)) {
            for (const path of installPathsOf(await $.fs.read(installed), plugin)) {
              candidates.push(...pluginFileCandidates(path, skill.name, plugin))
            }
          }
        }
        // A claude.ai-synced skill sits under an account directory only
        // `$.fs.list` can name.
        if (home) {
          const synced = `${home}/.claude/skills/synced`
          if (await $.fs.exists(synced)) {
            const accounts = (await $.fs.list(synced)).filter((entry) => entry.kind === 'dir').map((entry) => entry.name)
            candidates.push(...syncedFileCandidates(home, accounts, skill.name))
          }
        }
        for (const file of candidates) {
          if (await $.fs.exists(file)) {
            found = { path: file, markdown: await $.fs.read(file) }
            break
          }
        }
      } catch (error) {
        $.ui.log(`could not read /${skill.name}: ${String(error)}`)
      }
      files.set(skill.name, found)
      return found
    }
    const bodyOf = async (skill: Skill, plugin: string | undefined): Promise<string | null> =>
      (await fileOf(skill, plugin))?.markdown ?? null

    let commands: Awaited<ReturnType<typeof $.command.list>>
    try {
      commands = await $.command.list()
      if (!displayToId && commands.some((command) => !commandLike(command.name))) {
        const found: { dir: string; markdown: string }[] = []
        for (const root of [await $.session.cwd(), (await $.env.get('HOME')) ?? '']) {
          const dir = root && `${root}/.claude/skills`
          if (!dir || !(await $.fs.exists(dir))) continue
          for (const entry of await $.fs.list(dir)) {
            const file = `${dir}/${entry.name}/SKILL.md`
            if (entry.kind === 'dir' && (await $.fs.exists(file))) found.push({ dir: entry.name, markdown: await $.fs.read(file) })
          }
        }
        displayToId = displayIds(found)
      }
      commands = canonical(commands, displayToId ?? new Map())
    } catch (error) {
      $.ui.log(`could not list the skills: ${String(error)}`)
      return next(e)
    }
    // Loading the skill itself, the mod is not bound to what the engine would
    // list: a skill hidden with skillOverrides is still a candidate.
    const skills = catalog(commands, injectContent ? new Set() : listed, neverSuggested)
    if (skills.length === 0) {
      if (logDecisions) $.ui.log('no skill · 0 candidates')
      return next(e)
    }
    const pluginOf = new Map(commands.map((command) => [command.name, command.plugin ?? pluginPrefix(command.name)]))

    const skillGate = await loadGate()
    if (!skillGate) return next(e)

    // Request 1: rank everything, and ask skill-gate whether the prompt wants a skill at all.
    const startedAt = await $.clock.now()
    let wide: Wide | null = null
    const wideAnswer = await ask(wideQuestions(skillGate, skills), 'ranking', skillGate.model)
    if (wideAnswer) wide = readWide(wideAnswer, skillGate)

    // Request 2: re-read the shortlist with each skill's full text, and let
    // every candidate be rejected on its own.
    let rerank: Rerank | null = null
    let rerankAttempted = false
    // Before the listing has been seen, `$.command.list()` may name a skill
    // the model is not allowed to invoke; its own frontmatter tells.
    const barred: string[] = []
    if (rerankEnabled && wide && passesGate(wide, policy)) {
      const candidates: Candidate[] = []
      for (const skill of shortlistOf(wide, skills, policy.shortlist)) {
        const body = await bodyOf(skill, pluginOf.get(skill.name))
        if (!modelInvocable(body)) {
          barred.push(skill.name)
          continue
        }
        candidates.push({ ...skill, detail: detailOf(skill, body, excerptChars) })
      }
      if (candidates.length > 0) {
        rerankAttempted = true
        const answer = await ask(rerankQuestions(candidates), 'rerank', skillGate.model)
        if (answer) rerank = readRerank(answer, candidates)
      }
    }

    const offered = barred.length > 0 ? skills.filter((skill) => !barred.includes(skill.name)) : skills
    let decision = decide(wide, rerank, offered, policy, rerankAttempted)
    let pick = decision.name ? (skills.find((skill) => skill.name === decision.name) ?? null) : null
    // The winner's own frontmatter has the last word, whichever path picked it.
    if (pick && !barred.includes(pick.name) && !modelInvocable(await bodyOf(pick, pluginOf.get(pick.name)))) {
      barred.push(pick.name)
      decision = { name: null, reason: `/${pick.name} has disable-model-invocation` }
      pick = null
    }
    let block: string | null
    if (injectContent && pick) {
      const file = await fileOf(pick, pluginOf.get(pick.name))
      const projectDir = await $.session.cwd()
      block = injectionBlock(pick, file?.markdown ?? null, file?.path ?? null, projectDir, injected.has(pick.name))
      if (file) injected.add(pick.name)
    } else {
      block = suggestionBlock(pick, hideListing)
    }
    // The one line a prompt leaves in the log.
    if (logDecisions) $.ui.log(summaryOf(pick?.name ?? null, decision.reason, wide, rerank, (await $.clock.now()) - startedAt))
    if (!block) return next(e)
    // Attached on the way down: one block after the prompt as typed, read by
    // the model and never shown to the person.
    return next({ ...e, context: [...(e.context ?? []), block] })
  }).catch(($, e, next) => {
    // A suggestion is never worth a lost prompt: it goes through as typed.
    $.ui.log(`no suggestion: ${next.error.kind}`)
    return next(e)
  })

  // An injected skill lives in the conversation, not the process: `/clear`
  // or a resume starts another under the same worker, and a compaction may
  // summarize the block away. Either way the next pick goes in whole again.
  on('session.end', async ($, e, next) => {
    injected.clear()
    return next(e)
  })
  on('session.compact', async ($, e, next) => {
    if (!e.agentId) injected.clear()
    return next(e)
  }).catch(($, e, next) => next(e))

  on('skill.prompt', { skill: 'jeff-skill-suggestion:setup' }, async ($, e, next) => {
    // The plugin's own setup command: its markdown is a placeholder, and the
    // prompt the model reads is written here, from the roster as the engine
    // has it and the user settings as they are. The model does the editing
    // with its own tools, so the change shows as a diff and asks permission.
    const mode = /\brestore\b/i.test(e.text) ? 'restore' : 'apply'
    // No plan from a partial roster or unreadable settings: the edit would
    // hide too little, and the backup would save the wrong values.
    let commands: Awaited<ReturnType<typeof $.command.list>> = []
    try {
      commands = await $.command.list()
      if (!displayToId && commands.some((command) => !commandLike(command.name))) {
        const found: { dir: string; markdown: string }[] = []
        for (const root of [await $.session.cwd(), (await $.env.get('HOME')) ?? '']) {
          const dir = root && `${root}/.claude/skills`
          if (!dir || !(await $.fs.exists(dir))) continue
          for (const entry of await $.fs.list(dir)) {
            const file = `${dir}/${entry.name}/SKILL.md`
            if (entry.kind === 'dir' && (await $.fs.exists(file))) found.push({ dir: entry.name, markdown: await $.fs.read(file) })
          }
        }
        displayToId = displayIds(found)
      }
      commands = canonical(commands, displayToId ?? new Map())
    } catch (error) {
      $.ui.log(`setup: could not list the skills: ${String(error)}`)
      return next({ ...e, text: setupAborted(`the skills could not be listed (${String(error)})`) })
    }
    const home = (await $.env.get('HOME')) ?? '~'
    const settingsPath = `${home}/.claude/settings.json`
    const backupPath = `${home}/.claude/jeff-skill-suggestion.skill-overrides.backup.json`
    let json: string | null = null
    try {
      if (await $.fs.exists(settingsPath)) json = await $.fs.read(settingsPath)
    } catch (error) {
      $.ui.log(`setup: could not read ${settingsPath}: ${String(error)}`)
      return next({ ...e, text: setupAborted(`${settingsPath} exists but could not be read (${String(error)})`) })
    }
    const settings = readSkillSettings(json)
    const plan = setupPlan(commands, settings, new Set([SETUP_COMMAND]))
    // An earlier run's backup is reused only if it is one: a file that is
    // not this mod's, or is corrupt, is nothing restore could apply, so no
    // setup is built on top of it.
    let backupExists = false
    try {
      backupExists = await $.fs.exists(backupPath)
      if (backupExists && !validBackup(await $.fs.read(backupPath))) {
        $.ui.log(`setup: ${backupPath} is not a valid backup`)
        return next({
          ...e,
          text: setupAborted(
            `${backupPath} exists but is not a backup this mod wrote (expected {"skillOverrides": {...}, "disableBundledSkills": true|false|null}); ask the user to inspect it and move it away, or fix it, before running the setup again`,
          ),
        })
      }
    } catch (error) {
      $.ui.log(`setup: could not read ${backupPath}: ${String(error)}`)
      return next({ ...e, text: setupAborted(`${backupPath} could not be read (${String(error)})`) })
    }
    return next({ ...e, text: setupInstructions(mode, plan, settings, settingsPath, backupPath, backupExists) })
  })
}
