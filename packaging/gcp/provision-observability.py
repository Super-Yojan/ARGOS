#!/usr/bin/env python3
"""Create scoped observability credentials privately; never print token values."""
import argparse,hashlib,json,os,pathlib,re,secrets

def main():
 p=argparse.ArgumentParser();p.add_argument('--output',type=pathlib.Path,required=True);p.add_argument('--endpoint',required=True);p.add_argument('--rover',action='append',required=True);p.add_argument('--producer',action='append',required=True);args=p.parse_args()
 if not args.endpoint.startswith('https://'):p.error('HTTPS collector endpoint required')
 if any(not re.fullmatch(r'[A-Za-z0-9_.-]{1,128}',n) or n in ('.','..','reader') for n in args.producer):p.error('invalid producer identity')
 if any(not r.isdecimal() for r in args.rover):p.error('numeric rover IDs required')
 os.umask(0o077);args.output.mkdir(parents=True,exist_ok=True);args.output.chmod(0o700)
 config_file=args.output/'credentials.json';existing=json.loads(config_file.read_text()) if config_file.exists() else []
 for producer in [*args.producer,'reader']:
  token_file=args.output/(producer+'.token')
  if not token_file.exists():token_file.write_text(secrets.token_urlsafe(32)+'\n')
  token_file.chmod(0o600);token=token_file.read_text().strip()
  if not token:raise SystemExit('Existing credential is empty; restore it explicitly')
  scope={'producer_id':producer if producer!='reader' else '*','rovers':args.rover+['_producer'],'read':producer=='reader'}
  digest=hashlib.sha256(token.encode()).hexdigest();existing=[c for c in existing if c['token_sha256']!=digest];existing.append({'token_sha256':digest,'scope':scope})
  if producer!='reader':
   upload=args.output/(producer+'.upload.json');upload.write_text(json.dumps({'producer_id':producer,'endpoint':args.endpoint.rstrip('/'),'token':token}));upload.chmod(0o600)
 temporary=config_file.with_suffix('.tmp');temporary.write_text(json.dumps(existing,indent=2));temporary.chmod(0o600);temporary.replace(config_file)
 print(f'Provisioned {len(args.producer)} source credentials and one reader credential in a private directory.')
if __name__=='__main__':main()
