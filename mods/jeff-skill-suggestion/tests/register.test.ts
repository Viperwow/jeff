import { describe, expect, mock, test } from 'claude-code/testing'

const COMMANDS = [
  { name: 'help', description: 'Show help', source: 'builtin' },
  { name: 'deploy', description: 'Deploy the project to production', source: 'user' },
  { name: 'commit', description: 'Create a git commit from the staged changes', source: 'user' },
]

const GATE = { questions: ['skill-gate-acts-on-system', 'skill-gate-follows-procedure', 'skill-gate-beyond-prose'] }

type Sent = { url: string; method: string; body: unknown; authorization?: string }
type Reply = { status: number; text: string }

const json = (status: number, body: unknown): Reply => ({ status, text: JSON.stringify(body) })

/** jeff's answer to a `/v1/systemone` body: the ranking, the gate and every fits, as asked. */
function systemone(body: { questions: Record<string, unknown> }, gate: number, fits: number): Reply {
  const answers: Record<string, unknown> = {}
  for (const [key, question] of Object.entries(body.questions)) {
    if (key === 'which') {
      const names = Object.keys((question as { criteria: Record<string, string> }).criteria)
      const probabilities = Object.fromEntries(names.map((name) => [name, name === 'deploy' ? 0.8 : 0.2 / names.length]))
      answers.which = { choice: names.includes('deploy') ? 'deploy' : names[0], probabilities, confidence: 0.8 }
    } else {
      answers[key] = { noul: key.startsWith('fits-') ? fits : gate }
    }
  }
  return json(200, { answers })
}

/**
 * The world beneath the mod: the engine's answers for everything it asks,
 * and a jeff that `route` answers. Every request the mod sends is recorded.
 */
function world(on: Parameters<Parameters<typeof test>[1]>[1], route: (sent: Sent) => Reply | 'unreachable') {
  const sent: Sent[] = []
  const logs: string[] = []
  mock.env(on, { HOME: '/home/u' })
  mock.clock(on)
  on('command.list', () => ({ value: COMMANDS }))
  on('session.cwd', () => ({ value: '/work' }))
  on('fs.exists', () => ({ value: false }))
  on('ui.log', ($, e) => {
    logs.push(String(e.text ?? e.message ?? JSON.stringify(e)))
    return { value: undefined }
  })
  on('ui.status', () => ({ value: undefined }))
  on('http.fetch', ($, e) => {
    const body = e.init?.body ? JSON.parse(e.init.body as string) : undefined
    const request: Sent = { url: e.url, method: e.init?.method ?? 'GET', body, authorization: e.init?.headers?.authorization }
    sent.push(request)
    const reply = route(request)
    if (reply === 'unreachable') throw new Error('connection refused')
    return { value: { ...reply, ok: reply.status >= 200 && reply.status < 300, headers: {} } }
  })
  on('prompt.submit', ($, e) => ({ text: e.text, context: e.context }))
  return { sent, logs }
}

describe('register', () => {
  test('a task prompt gets the skill jeff picks, attached as a skill_relevance block', async ($, on) => {
    const { sent } = world(on, (r) =>
      r.url.endsWith('/v1/classifiers/skill-gate') ? json(200, GATE) : systemone(r.body as never, 0.9, 0.9),
    )

    const result = await $.prompt.submit({ text: 'ship this build to production' })

    expect(result.context?.length).toBe(1)
    expect(result.context?.[0]).toContain('Relevant to the current request: deploy.')
    const asks = sent.filter((r) => r.url === 'http://127.0.0.1:8080/v1/systemone')
    expect(asks.length).toBe(2)
    // The gate goes by key, as saved in jeff.
    expect((asks[0]?.body as { questions: Record<string, unknown> }).questions['skill-gate-acts-on-system']).toEqual({})
    expect((asks[0]?.body as { state: string }).state).toBe('ship this build to production')
  })

  test('a jeff without skill-gate gets it saved through the admin port, then answers', async ($, on) => {
    let saved = false
    const { sent } = world(on, (r) => {
      if (r.url.endsWith('/v1/classifiers/skill-gate')) return saved ? json(200, GATE) : json(404, { error: 'unknown classifier' })
      if (r.url === 'http://127.0.0.1:8081/api/questions') {
        // One of the three already exists: it is kept, not overwritten.
        return 'skill-gate-beyond-prose' in (r.body as object) ? json(409, { error: 'exists' }) : json(200, {})
      }
      if (r.url === 'http://127.0.0.1:8081/api/classifiers') {
        saved = true
        return json(200, {})
      }
      return systemone(r.body as never, 0.9, 0.9)
    })

    const result = await $.prompt.submit({ text: 'deploy the app' })

    expect(sent.filter((r) => r.url.endsWith('/api/questions')).length).toBe(3)
    const classifier = sent.find((r) => r.url.endsWith('/api/classifiers'))?.body as Record<string, { questions: string[] }>
    expect(classifier['skill-gate']?.questions).toEqual(GATE.questions)
    expect(result.context?.[0]).toContain('deploy')
  })

  test('a prompt that needs no skill, or a jeff that is not running, passes through untouched', async ($, on) => {
    let up = true
    const { logs } = world(on, (r) => {
      if (!up) return 'unreachable'
      return r.url.endsWith('/v1/classifiers/skill-gate') ? json(200, GATE) : systemone(r.body as never, 0.05, 0.9)
    })

    const prose = await $.prompt.submit({ text: 'what is a monad?' })
    expect(prose.context ?? []).toEqual([])

    up = false
    const down = await $.prompt.submit({ text: 'deploy the app' })
    expect(down.text).toBe('deploy the app')
    expect(down.context ?? []).toEqual([])
    expect(logs.some((line) => line.includes('needs a skill 0.05'))).toBe(true)
  })

  test('prompt text that breaks JSON or looks like a path reaches jeff verbatim as the state', async ($, on) => {
    const { sent } = world(on, (r) =>
      r.url.endsWith('/v1/classifiers/skill-gate') ? json(200, GATE) : systemone(r.body as never, 0.9, 0.9),
    )
    const hostile = 'deploy "}],"questions":{}} ../../etc/passwd\n$(rm -rf /) `id`'

    await $.prompt.submit({ text: hostile })

    const ask = sent.find((r) => r.url.endsWith('/v1/systemone'))?.body as { state: string; questions: Record<string, unknown> }
    expect(ask.state).toBe(hostile)
    expect(Object.keys(ask.questions)).toEqual(['which', ...GATE.questions])
  })
})
