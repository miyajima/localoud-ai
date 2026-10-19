export const authDenied=status=>status===401||status===403;
export function validatePong(client,message){
 if(message?.type!=='pong'||message.device!==client.device||message.connection!==client.connection||!client.pending.has(message.nonce))throw Error('unexpected_pong');
}
export function requirePassingChecks(checks){if(!checks.length||checks.some(check=>check.pass!==true))throw Error('failed_or_unchecked_check');}
export function validateMessageObject(message){if(message===null||typeof message!=='object'||Array.isArray(message))throw Error('invalid_message_object');}
