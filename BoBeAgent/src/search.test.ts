import assert from "node:assert/strict";
import test from "node:test";
import { mkdir,writeFile } from "node:fs/promises";
import { join } from "node:path";
import { configuration,fixture,host } from "./test-support.ts";
import {fileURLToPath} from "node:url";

test("grep and find retain pi search results but launch through the daemon process owner",async t=>{
  const paths=await fixture(t);
  await mkdir(join(paths.workspace,"src"));
  await writeFile(join(paths.workspace,"src/notes.txt"),"before\nNeedle one\nbetween\nneedle two\nafter\n");
  let calls=0;
  let results="";
  const {worker,processes}=await host(t,configuration(paths),context=>{
    if (calls++===0) return {tools:[
      {id:"grep",name:"grep",arguments:{pattern:"needle",path:"src",ignoreCase:true,context:1,limit:1}},
      {id:"find",name:"find",arguments:{pattern:"src/*.txt"}},
    ]};
    results=JSON.stringify(context.messages.filter(message=>message.role==="toolResult"));
    return {text:"Both searches completed."};
  });

  await worker.request({id:2,method:"chat",params:{text:"Search",channel:"text",context:""}});
  assert.ok(processes.programs.some(program=>program.endsWith("/rg")));
  assert.ok(processes.programs.some(program=>program.endsWith("/fd") || program.endsWith("/fdfind")));
  assert.ok(results.includes("notes.txt:2: Needle one"),results);
  assert.ok(results.includes("notes.txt-1- before"),results);
  assert.ok(results.includes("1 matches limit reached"),results);
  assert.ok(results.includes("src/notes.txt"),results);
});

test("grep keeps regex semantics when the daemon environment names a ripgrep config",async t=>{
  const paths=await fixture(t);
  await writeFile(join(paths.workspace,"notes.txt"),"alpha 42\n");
  const config=join(paths.root,"ripgreprc");
  await writeFile(config,"--fixed-strings\n");
  const previous=process.env.RIPGREP_CONFIG_PATH;
  process.env.RIPGREP_CONFIG_PATH=config;
  t.after(()=>{
    if (previous===undefined) delete process.env.RIPGREP_CONFIG_PATH;
    else process.env.RIPGREP_CONFIG_PATH=previous;
  });
  let calls=0;
  let results="";
  const {worker}=await host(t,configuration(paths),context=>{
    if (calls++===0) return {tools:[{id:"grep",name:"grep",arguments:{pattern:"alpha \\d+"}}]};
    results=JSON.stringify(context.messages.filter(message=>message.role==="toolResult"));
    return {text:"Searched."};
  });
  await worker.request({id:2,method:"chat",params:{text:"Search",channel:"text",context:""}});
  assert.ok(results.includes("notes.txt:1: alpha 42"),results);
});

test("persistent MCP processes and native search share the same process capacity",async t=>{
  const paths=await fixture(t);
  const config=configuration(paths);
  config.mcp={servers:Object.fromEntries([0,1,2,3].map(index=>[`server${index}`,{
    command:process.execPath,args:[fileURLToPath(new URL("./mcp-fixture.ts",import.meta.url))],
    env:{},timeout_ms:15_000,excluded_tools:[],
  }]))};
  let calls=0;
  let results="";
  const {worker,processes,notifications}=await host(t,config,context=>{
    if (calls++===0) return {tools:[{id:"search",name:"find",arguments:{pattern:"*.txt"}}]};
    results=JSON.stringify(context.messages.filter(message=>message.role==="toolResult"));
    return {text:"Search could not start because the process budget was occupied."};
  },undefined,undefined,4);
  await worker.request({id:2,method:"chat",params:{text:"Search",channel:"text",context:""}});
  assert.ok(results.includes("process capacity is full"),results);
  assert.equal(processes.programs.length,4);
  assert.ok(notifications.some(event=>event.method==="event" && event.params.delta.type==="tool_complete" && !event.params.delta.success));
});
