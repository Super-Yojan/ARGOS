import type { Identity } from './generated';
// Per-tab identity: a duplicated tab must acquire its own command claim.
export const clientID=crypto.randomUUID();
export class ApiError extends Error {constructor(public status:number,message:string){super(message)}}
export async function api<T>(path:string,user?:Identity,method='GET',body?:unknown):Promise<T>{
  const response=await fetch(`/api/v1${path}`,{method,credentials:'same-origin',headers:{'Content-Type':'application/json','x-argos-client':clientID,...(user?{'x-csrf-token':user.csrf}:{})},body:body===undefined?undefined:JSON.stringify(body)});
  if(!response.ok){const message=await response.json().catch(()=>({error:`Request failed (${response.status})`}));throw new ApiError(response.status,message.error??`Request failed (${response.status})`)}
  if(response.status===204)return undefined as T;
  return response.json() as Promise<T>;
}
