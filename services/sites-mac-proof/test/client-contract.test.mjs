import test from 'node:test';
import assert from 'node:assert/strict';
import {authDenied, validatePong, requirePassingChecks, validateMessageObject} from '../client/protocol.mjs';
test('only actual auth denial establishes privacy',()=>{for(const status of [401,403])assert.equal(authDenied(status),true);for(const status of [200,302,429,500,503])assert.equal(authDenied(status),false);});
test('pong must match pending nonce, device and current connection',()=>{const c={device:'A',connection:'C',pending:new Map([['n',{}]])};assert.doesNotThrow(()=>validatePong(c,{type:'pong',device:'A',connection:'C',nonce:'n'}));for(const patch of [{nonce:'unknown'},{device:'B'},{connection:'old'},{type:'ready'}])assert.throws(()=>validatePong(c,{type:'pong',device:'A',connection:'C',nonce:'n',...patch}));c.pending.delete('n');assert.throws(()=>validatePong(c,{type:'pong',device:'A',connection:'C',nonce:'n'}));});
test('completed requires every check to pass',()=>{assert.doesNotThrow(()=>requirePassingChecks([{pass:true}]));for(const checks of [[{pass:false}],[{name:'unchecked'}],[]])assert.throws(()=>requirePassingChecks(checks));});

test('null and primitive messages fail before field access',()=>{for(const value of [null,[],0,'pong',false])assert.throws(()=>validateMessageObject(value));assert.doesNotThrow(()=>validateMessageObject({type:'pong'}));});
