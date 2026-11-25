import {createInterface} from "node:readline";

const args = process.argv.slice(2);
const input = createInterface({input:process.stdin});
const id = "d637f227-050d-484d-bfa7-d43678be869a";
const cwd = process.env.NAROLINK_TEST_WORKSPACE;
let selected = "default";
let pending = false;
let waiting = null;
const send = frame => process.stdout.write(JSON.stringify(frame) + "\n");
const notify = (method, params) => send({method, params});
const acp = (method, params) => send({jsonrpc:"2.0", method, params});
const reply = (frame, result) => send({id:frame.id, result});
const open = () => ({sessionId:id, configOptions:[{
  id:"model", category:"model", currentValue:"default",
  options:[{value:"model-two",name:"Model Two"}],
}]});
const update = (sessionUpdate, text) => acp("session/update", {
  sessionId:id, update:{sessionUpdate,content:{type:"text",text}},
});

function requirePng(encoded) {
  if (typeof encoded !== "string" ||
      Buffer.from(encoded, "base64").subarray(0,8).toString("hex") !== "89504e470d0a1a0a") {
    throw new Error("A normalized PNG must arrive in the agent's image frame");
  }
  if (process.env.NAROLINK_RELAY_TOKEN !== undefined) throw new Error("Relay credential was inherited by the CLI");
}
function rpc(frame) {
  const params = frame.params || {};
  switch (frame.method) {
    case "initialize": return reply(frame,{protocolVersion:1,agentCapabilities:{loadSession:true}});
    case "thread/list": return reply(frame,{data:[{id,cwd,name:"Native Codex History",updatedAt:1700000000}]});
    case "thread/items/list": return reply(frame,{data:[
      {item:{type:"agentMessage",text:"Existing Codex response"}},
      {item:{type:"userMessage",content:[{type:"text",text:"Original Codex prompt"}]}},
    ]});
    case "thread/resume":
      if (params.threadId !== id || params.excludeTurns !== true) throw new Error("Original Codex ID required");
      selected = params.model;
      return reply(frame,{thread:{id}});
    case "thread/start": return reply(frame,{thread:{id}});
    case "model/list": return reply(frame,{data:[{id:"model-two",model:"model-two",displayName:"Model Two"}]});
    case "turn/start":
      requirePng(params.input.find(item => item.type === "image")?.url?.split(",")[1]);
      if (params.cwd !== cwd || params.sandboxPolicy.type !== "workspaceWrite") throw new Error("Workspace was not preserved");
      if (params.model) selected = params.model;
      reply(frame,{turn:{id:"t1"}});
      notify("item/agentMessage/delta",{threadId:id,itemId:"m1",delta:"Model " + selected});
      notify("item/completed",{threadId:id,item:{id:"m1",type:"agentMessage",text:"Model " + selected + " confirmed"}});
      if (params.input[0].text !== "wait until cancelled")
        return notify("turn/completed",{threadId:id,turn:{status:"completed"}});
      return;
    case "session/list":
      return reply(frame,{sessions:[{sessionId:id,cwd,title:"Native Antigravity History",updatedAt:"2025-08-10T15:36:00Z"}]});
    case "session/load":
      if (params.sessionId !== id || params.cwd !== cwd) throw new Error("Wrong ACP history");
      update("user_message_chunk","Original Antigravity prompt");
      update("agent_message_chunk","Existing Antigravity response");
      return reply(frame,open());
    case "session/resume":
      if (params.sessionId !== id || params.cwd !== cwd) throw new Error("Wrong ACP resume");
      return reply(frame,open());
    case "session/new": return reply(frame,open());
    case "session/set_config_option":
      if (params.sessionId !== id || params.configId !== "model") throw new Error("Wrong ACP model selection");
      selected = params.value;
      return reply(frame,open());
    case "session/prompt":
      requirePng(params.prompt.find(item => item.type === "image")?.data);
      waiting = frame;
      update("agent_message_chunk","Model " + selected);
      send({jsonrpc:"2.0",id:71,method:"session/request_permission",params:{
        sessionId:id,toolCall:{title:"Review sample edit",rawInput:{path:"sample.txt"}},
        options:[{kind:"allow_once",optionId:"yes"},{kind:"reject_once",optionId:"no"}],
      }});
      return;
    default:
      if (waiting && frame.id === 71) {
        update("agent_message_chunk", frame.result.outcome.optionId === "yes" ? " approved" : " declined");
        reply(waiting,{stopReason:"end_turn"});
        waiting = null;
      }
  }
}

function claude(frame) {
  if (frame.type === "control_request" && frame.request.subtype === "initialize") {
    return send({type:"control_response",response:{request_id:frame.request_id,subtype:"success",response:{
      models:[{value:"model-two",displayName:"Model Two"}],
    }}});
  }
  if (frame.type === "user") {
    requirePng(frame.message.content.find(item => item.type === "image")?.source?.data);
    if (args[args.indexOf("--resume") + 1] !== id || frame.session_id !== id)
      throw new Error("Claude Code original session is required");
    selected = args[args.indexOf("--model") + 1];
    send({type:"system",subtype:"init",session_id:id});
    send({type:"stream_event",event:{type:"message_start",message:{id:"c1"}}});
    send({type:"stream_event",event:{type:"content_block_delta",index:0,delta:{type:"text_delta",text:"Model "}}});
    send({type:"assistant",message:{id:"c1",content:[{type:"text",text:"Model " + selected}]}});
    send({type:"control_request",request_id:"action",request:{subtype:"can_use_tool",tool_name:"Edit",input:{file_path:"sample.txt"}}});
    pending = true;
  }
  if (pending && frame.type === "control_response") {
    pending = false;
    send({type:"assistant",message:{id:"c2",content:[{type:"text",
      text:frame.response.response.behavior === "allow" ? "approved" : "declined"}]}});
    send({type:"result",subtype:"success",session_id:id});
  }
}

for await (const line of input) {
  const frame = JSON.parse(line);
  if (args.includes("--print")) claude(frame);
  else rpc(frame);
}
