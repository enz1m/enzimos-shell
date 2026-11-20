import json,sys

def send(v):print(json.dumps(v),flush=True)
send({'type':'hello','protocol':1,'plugin':'peer-fixture'});initial=json.loads(sys.stdin.readline());count=0;revision=0

def snapshot():
 global revision
 revision+=1
 send({'type':'snapshot','revision':revision,'elements':[{'id':'peer','icon':'plus','label':f'Peer {count}','tooltip':'Disposable healthy peer','panel':{'title':'Healthy peer','width':360,'height':300,'nodes':[{'type':'text','key':'count','text':f'Count: {count}','tone':'title'},{'type':'action','key':'increment','id':'increment','label':'Increment peer','detail':None,'icon':None,'thumbnail':None,'enabled':True}]}}]})
snapshot()
for line in sys.stdin:
 message=json.loads(line)
 if message['type']=='ping':send({'type':'pong','sequence':message['sequence']})
 elif message['type']=='action':count+=1;snapshot()
