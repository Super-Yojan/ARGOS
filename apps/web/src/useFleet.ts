import { useEffect,useState } from 'react';
import { clientID } from './api';
import type { FleetSnapshot,Identity } from './generated';
export function useFleet(user:Identity|null,onDisconnect:()=>void){
  const [fleet,setFleet]=useState<FleetSnapshot|null>(null);
  const [connected,setConnected]=useState(false);
  useEffect(()=>{
    if(!user){setFleet(null);setConnected(false);return}
    let received=Date.now(),cleared=false,disposed=false,events:EventSource|null=null;
    const disconnect=()=>{setConnected(false);if(!cleared){cleared=true;onDisconnect()}};
    function open(){
      if(disposed||!navigator.onLine)return;
      events?.close();received=Date.now();setConnected(false);
      const source=new EventSource(`/api/v1/events?client=${clientID}`);
      events=source;
      source.addEventListener('fleet',event=>{
        if(disposed||events!==source)return;
        try{const next=JSON.parse((event as MessageEvent).data) as FleetSnapshot;setFleet(next);received=Date.now();cleared=false;setConnected(true)}catch{disconnect()}
      });
      source.onerror=()=>{if(!disposed&&events===source)disconnect()};
    }
    const offline=()=>{events?.close();events=null;disconnect()};
    if(navigator.onLine)open();else disconnect();
    window.addEventListener('offline',offline);window.addEventListener('online',open);
    const recover=()=>{if(Date.now()-received>3000){disconnect();if(navigator.onLine)open()}};
    const visible=()=>{if(document.visibilityState==='visible')recover()};
    document.addEventListener('visibilitychange',visible);window.addEventListener('focus',recover);
    const timer=setInterval(recover,500);
    return()=>{disposed=true;events?.close();events=null;clearInterval(timer);window.removeEventListener('offline',offline);window.removeEventListener('online',open);document.removeEventListener('visibilitychange',visible);window.removeEventListener('focus',recover)};
  },[user,onDisconnect]);
  return {fleet,connected};
}
