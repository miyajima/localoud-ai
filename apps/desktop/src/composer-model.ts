export type Mention = {kind:'skill'|'plugin'|'file';name:string;path:string}|{kind:'agent';name:string};
export type Entry = {kind:string;name:string;label:string;description:string;path:string};
export type Trigger = {start:number;end:number;sigil:'/'|'@';query:string};
export function triggerAt(text:string,cursor:number):Trigger|null {
  const before=text.slice(0,cursor),match=/(?:^|\s)([/@])([^\s]*)$/.exec(before);
  if(!match)return null;
  return {start:cursor-match[2].length-1,end:cursor,sigil:match[1] as '/'|'@',query:match[2]};
}
export function historySuffix(prefix:string,history:string[]):string|null {
  if(prefix.trim().length<2)return null;
  const match=history.find(text=>text.startsWith(prefix)&&text.length>prefix.length);
  return match?match.slice(prefix.length):null;
}
export function mentionKey(mention:Mention){return JSON.stringify(mention);}
export function filterEntries(entries:Entry[],query:string,kind?:string):Entry[]{
  const needle=query.toLocaleLowerCase();
  return entries.filter(e=>(!kind||e.kind===kind)&&`${e.name} ${e.label} ${e.description}`.toLocaleLowerCase().includes(needle)).slice(0,30);
}
export function canAcceptSuffix(original:string,current:string,originalContext:string,currentContext:string,cursor:number){
  return original===current&&originalContext===currentContext&&cursor===current.length;
}
