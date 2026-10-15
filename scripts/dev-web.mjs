// No shell-specific environment assignment: works in PowerShell, cmd, zsh and bash.
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
const cwd=fileURLToPath(new URL('..',import.meta.url));
const children=[];
let stopping=false;
function stop(code=0){
  if(stopping)return;stopping=true;
  for(const child of children){
    if(process.platform==='win32')spawn('taskkill',['/pid',String(child.pid),'/T','/F'],{stdio:'ignore'});
    else child.kill('SIGTERM');
  }
  setTimeout(()=>process.exit(code),300).unref();
}
function run(command,args){
  const child=spawn(command,args,{cwd,stdio:'inherit',env:process.env});children.push(child);
  child.on('error',error=>{console.error(`${command}: ${error.message}`);stop(1)});
  child.on('exit',code=>stop(code??1));
}
run('cargo',['run','-p','argos-server','--','--demo','--dev']);
if(process.env.npm_execpath)run(process.execPath,[process.env.npm_execpath,'--prefix','apps/web','run','dev']);
else if(process.platform==='win32')run('cmd.exe',['/d','/s','/c','npm --prefix apps/web run dev']);
else run('npm',['--prefix','apps/web','run','dev']);
process.on('SIGINT',()=>stop());process.on('SIGTERM',()=>stop());
