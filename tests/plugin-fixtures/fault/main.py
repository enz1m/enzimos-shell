import json,sys,os,time

def send(v):print(json.dumps(v),flush=True)
send({'type':'hello','protocol':1,'plugin':'fault-fixture'});initial=json.loads(sys.stdin.readline());revision=0;request=0;total=0

def subscribe():
 global request
 request+=1;send({'type':'request','id':request,'request':{'operation':'notifications.subscribe','enabled':True}})

def snapshot(invalid=False):
 global revision
 revision+=1
 nodes=[{'type':'text','key':'state','text':f'Owned fixture; current feed count {total}.','tone':'muted'}]
 for key,label in [('crash','Crash worker'),('hang','Hang worker'),('flood','Flood requests'),('oversized','Oversized frame'),('invalid','Invalid UI'),('allocate','Excess allocation'),('authority','Forbidden notification action')]:
  nodes.append({'type':'action','key':key,'id':key,'label':label,'detail':None,'icon':None,'thumbnail':None,'enabled':True})
 if invalid:nodes[0]['type']='forbidden-node'
 send({'type':'snapshot','revision':revision,'elements':[{'id':'fault','icon':'flask-conical','label':'Fault fixture','tooltip':'Disposable owned fault fixture','panel':{'title':'Owned fault fixture','width':400,'height':620,'nodes':nodes}}]})
subscribe();snapshot()
for line in sys.stdin:
 message=json.loads(line)
 if message['type']=='ping':send({'type':'pong','sequence':message['sequence']})
 elif message['type']=='provider':
  if message['data'].get('provider')=='notifications':total=message['data'].get('total',0);snapshot()
 elif message['type']=='action':
  action=message['action']
  if action=='crash':os._exit(17)
  elif action=='hang':
   while True:time.sleep(60)
  elif action=='flood':
   for _ in range(1000):subscribe()
  elif action=='oversized':sys.stdout.write('X'*65538+'\n');sys.stdout.flush()
  elif action=='invalid':snapshot(True)
  elif action=='allocate':allocation=[bytearray(1024*1024) for _ in range(96)]
  elif action=='authority':
   request+=1;send({'type':'request','id':request,'request':{'operation':'notifications.invoke','id':'notice:fixture:1','action':'default'}})
