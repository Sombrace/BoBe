import assert from "node:assert/strict";
import test from "node:test";
import { randomUUID } from "node:crypto";
import { configuration,fixture,host } from "./test-support.ts";
import { isRecord } from "./protocol.ts";
import {access} from "node:fs/promises";
import {join} from "node:path";

for (const background of [false,true]) {
  test(`an important ${background ? "background" : "chat"} question ends native execution without a pending UI promise`,async t=>{
    const paths=await fixture(t);
    let calls=0;
    let requests=0;
    const prompt="Email the reviewed draft to Pat.\n\nSend this draft?\n\nYes, No";
    const {worker,notifications}=await host(t,configuration(paths),()=>{
      calls++;
      return {tools:[{id:"ask-1",name:"bobe_owner_input",arguments:{
        operation:"ask",kind:"approval",question:"Send this draft?",action:"Email the reviewed draft to Pat.",
        task:"Inspect and send the reviewed draft if approved.",
      }}]};
    },async (method,params)=>{
      assert.equal(method,"owner_input");
      assert.equal(params.name,"ask");
      requests++;
      return {id:randomUUID(),prompt,status:"pending"};
    });
    const result=await worker.request(background
      ? {id:2,method:"background",params:{job_id:randomUUID(),title:"Draft",task:"Prepare a draft",context:"",memory_context:"",persona:{souls:[],profiles:[]},attempt:1,
          route:{profile:"test",model:"fixture-model",reasoning_effort:"low"},budget:{timeout_seconds:1800,max_turns:64}}}
      : {id:2,method:"chat",params:{text:"Prepare a draft",channel:"text",context:""}});
    assert.equal(calls,1);
    assert.equal(requests,1);
    if (background) {assert.ok(isRecord(result)); assert.equal(result.output,prompt);}
    else {
      assert.ok(notifications.some(event=>event.method==="event" && event.params.delta.type==="message_complete" && event.params.delta.content===prompt));
      assert.ok(notifications.some(event=>event.method==="event" && event.params.delta.type==="done"));
    }

  });
}

test("an important question blocks effects requested in the same parallel tool round",async t=>{
  const paths=await fixture(t);
  let calls=0;
  let requests=0;
  let observed="";
  const {worker}=await host(t,configuration(paths),context=>{
    if (calls++>0) {
      observed=JSON.stringify(context.messages.filter(message=>message.role==="toolResult"));
      return {text:"Stopped."};
    }
    return {tools:[
      {id:"write-before-question",name:"write",arguments:{path:"must-not-exist.txt",content:"unapproved"}},
      {id:"ask",name:"bobe_owner_input",arguments:{
        operation:"ask",kind:"approval",question:"Create this file?",action:"Write the requested file",
        task:"Write it only if approved",
      }},
    ]};
  },async()=>{requests++;return {id:randomUUID(),prompt:"Create this file?",status:"pending"};});
  await worker.request({id:2,method:"chat",params:{text:"Decide first",channel:"text",context:""}});
  assert.equal(requests,1,observed);
  assert.equal(calls,1,observed);
  await assert.rejects(access(join(paths.workspace,"must-not-exist.txt")),error=>isRecord(error)&&error.code==="ENOENT");
});
