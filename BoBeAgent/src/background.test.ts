import assert from "node:assert/strict";
import test from "node:test";
import { setTimeout as delay } from "node:timers/promises";
import { randomUUID } from "node:crypto";
import { readFile, readdir } from "node:fs/promises";
import { join } from "node:path";
import { getCurrentSystemMessage, getCurrentTools } from "@earendil-works/pi-ai";

import { configuration, fixture, host, userText } from "./test-support.ts";
import { isRecord } from "./protocol.ts";

test("background execution does not occupy the canonical chat and has no recursive job authority", async t => {
  const paths = await fixture(t);
  const started = Promise.withResolvers<void>();
  const finished = Promise.withResolvers<void>();
  const { worker, notifications } = await host(t, configuration(paths), async context => {
    if (userText(context) === "Background work") {
      started.resolve();
      const declared = JSON.stringify(context);
      assert.ok(!declared.includes('"name":"background_task"'));
      assert.equal(getCurrentSystemMessage(context.messages)?.sections?.bobe_persona,
        "<bobe_persona>\n<soul>\nBACKGROUND_SOUL\n</soul>\n</bobe_persona>");
      await finished.promise;
      return { text: "Background result." };
    }
    return { text: "The companion remains available." };
  });
  const pointer = await readFile(join(paths.root, "agent/session.json"), "utf8");
  const job = randomUUID();
  const running = worker.request({
    id: 2, method: "background", params: {
      job_id: job, title: "Bounded task", task: "Background work", context: "", memory_context: "",
      persona: { souls: ["BACKGROUND_SOUL"], profiles: [] }, attempt: 1, route: { profile: "test", model: "fixture-model", reasoning_effort: "low" },
      budget: { timeout_seconds: 1800, max_turns: 64 },
    },
  });
  await started.promise;
  await worker.request({ id: 3, method: "chat", params: { text: "Hello", channel: "text", context: "" } });
  assert.ok(notifications.some(event => event.method === "event" && event.params.request_id === 3 && event.params.delta.type === "done"));
  assert.ok(!notifications.some(event => event.method === "event" && event.params.request_id === 2));
  finished.resolve();
  const result = await running;
  assert.ok(isRecord(result) && result.output === "Background result.");
  assert.equal(await readFile(join(paths.root, "agent/session.json"), "utf8"), pointer);
  assert.ok((await readdir(join(paths.root, "agent/sessions/jobs", job, "sessions"))).length === 1);
});

test("aborting a background job settles only that job and permits a later foreground turn", async t => {
  const paths = await fixture(t);
  const started = Promise.withResolvers<void>();
  const { worker } = await host(t, configuration(paths), async (context, options) => {
    if (userText(context) === "Wait") {
      started.resolve();
      await delay(30_000, undefined, { signal: options?.signal });
    }
    return { text: "Ready." };
  });
  const running = worker.request({ id: 2, method: "background", params: {
    job_id: randomUUID(), title:"Wait",task:"Wait",context:"",memory_context:"",persona:{souls:[],profiles:[]},attempt:1,
    route:{profile:"test",model:"fixture-model",reasoning_effort:null},
    budget:{timeout_seconds:1800,max_turns:64},
  } });
  const rejected = assert.rejects(running);
  await started.promise;
  await worker.request({ id: 3, method:"abort", params:{request_id:2} });
  await rejected;
  await worker.request({ id:4,method:"chat",params:{text:"Hello",channel:"text",context:""} });
});

test("disabled native tools stay unavailable after resuming a session that once had them", async t => {
  const paths = await fixture(t);
  const config = configuration(paths);
  const first = await host(t, config, () => ({ text: "Initial session." }));
  await first.worker.request({id:2,method:"chat",params:{text:"Hello",channel:"text",context:""}});
  await first.worker.close();
  config.enabled_tools=["read", "subagent"];
  let calls = 0;
  const second = await host(t, config, context => {
    assert.ok(!getCurrentTools(context.messages).some(tool => tool.name === "bash"));
    return ++calls === 1
      ? {tools:[{id:"disabled-bash",name:"bash",arguments:{command:"must-not-run"}}]}
      : {text:"Shell is unavailable."};
  });
  await second.worker.request({id:2,method:"chat",params:{text:"Check tools",channel:"text",context:""}});
  assert.ok(second.notifications.some(event => event.method==="event" && event.params.delta.type==="tool_complete" && !event.params.delta.success));
});

test("background creation returns a durable source identity without waiting for execution", async t => {
  const paths=await fixture(t);
  let calls=0;
  let source: unknown;
  const {worker}=await host(t,configuration(paths),()=>++calls===1
    ? {tools:[{id:"source-tool",name:"background_task",arguments:{title:"Useful work",task:"Do bounded work",context:"Explicit scope"}}]}
    : {text:"The task is queued."},
    async (method,params) => {
      assert.equal(method,"background_job");
      assert.equal(params.name,"start");
      assert.equal(params.request_id,2);
      source=params.source_key;
      return {id:randomUUID(),status:"queued"};
    });
  await worker.request({id:2,method:"chat",params:{text:"Do useful work in the background",channel:"text",context:""}});
  assert.ok(typeof source==="string" && /^[0-9a-f]{64}$/.test(source));
});

test("provider-local tool IDs do not collapse separate task creations", async t => {
  const paths=await fixture(t);
  const seen=new Set<string>();
  const sources:string[]=[];
  const { worker }=await host(t,configuration(paths),context => {
    const text=userText(context);
    if (seen.has(text)) return { text:"Queued." };
    seen.add(text);
    return { tools:[{id:"call_0",name:"background_task",arguments:{title:text,task:text,context:""}}] };
  },async (method,params) => {
    assert.equal(method,"background_job");
    assert.equal(typeof params.source_key,"string");
    sources.push(String(params.source_key));
    return { id:randomUUID(),status:"queued" };
  });
  for (const [id,text] of [[2,"first task"],[3,"second task"]] as const) {
    await worker.request({id,method:"chat",params:{text,channel:"text",context:""}});
  }
  assert.equal(new Set(sources).size,2);
});

test("a real pi background session can plan a durable future wake without spawning a recursive live job",async t => {
  const paths=await fixture(t);
  let calls=0;
  let schedules=0;
  const { worker }=await host(t,configuration(paths),() => ++calls===1
    ? {tools:[{id:"call_0",name:"bobe_schedule",arguments:{
      action:"create",title:"Later review",task:"Inspect the next result",context:"bounded continuation",
      at:"2026-11-01T10:00:00Z",every_seconds:3600,
    }}]}
    : {text:"The next review is scheduled."},
    async (method,params) => {
      assert.equal(method,"schedule");
      assert.equal(params.name,"create");
      assert.equal(params.request_id,2);
      assert.ok(typeof params.source_key==="string" && /^[a-f0-9]{64}$/.test(params.source_key));
      schedules++;
      return {id:randomUUID(),enabled:true};
    },
  );
  await worker.request({id:2,method:"background",params:{
    job_id:randomUUID(),title:"Plan",task:"Plan the next check",context:"",memory_context:"",persona:{souls:[],profiles:[]},attempt:1,
    route:{profile:"test",model:"fixture-model",reasoning_effort:"low"},
    budget:{timeout_seconds:1800,max_turns:64},
  }});
  assert.equal(schedules,1);
});

test("tool exclusions also apply to scoped children and persistent background sessions", async t => {
  const paths = await fixture(t);
  const config = configuration(paths);
  config.enabled_tools = ["read", "subagent", "background_jobs"];
  const seen = new Set<string>();
  let parentCalls = 0;
  const { worker } = await host(t, config, context => {
    const tools = getCurrentTools(context.messages);
    assert.ok(!tools.some(tool => ["bash", "edit", "write", "grep", "find", "ls"].includes(tool.name)));
    const text = userText(context);
    seen.add(text);
    if (text === "Parent" && parentCalls++ === 0) {
      return { tools: [{ id: "child", name: "subagent", arguments: {
        mode: "parallel", tasks: [{ task: "Child" }],
      } }] };
    }
    return { text: "Read-only work completed." };
  });
  await worker.request({ id: 2, method: "chat", params: { text: "Parent", channel: "text", context: "" } });
  const job = randomUUID();
  const params = {
    job_id: job, title: "Background", task: "Background", context: "", memory_context: "",
    persona: { souls: [], profiles: [] }, attempt: 1,
    route: { profile: "test", model: "fixture-model", reasoning_effort: null },
    budget: { timeout_seconds: 1800, max_turns: 64 },
  };
  const first = await worker.request({ id: 3, method: "background", params });
  const resumed = await worker.request({ id: 4, method: "background", params: { ...params, attempt: 2 } });
  assert.ok(isRecord(first) && isRecord(resumed));
  assert.equal(first.session_id, resumed.session_id);
  assert.ok(seen.has("Parent") && seen.has("Child") && seen.has("Background"));
});
