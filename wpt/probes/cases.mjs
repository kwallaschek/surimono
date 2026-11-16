// Small pages for what WPT's slices don't pin: each gap and spec conflict in
// docs/COMPAT.md that cites `wpt/probes` has a case here, by id. A case calls
// log(); run.mjs runs it in the pinned Chromium and in surimono and writes both
// answers to results.json. Nothing here is an expectation: Chrome's answer is the
// reference, and where the spec differs the COMPAT row says so.
// A body that starts with <html is a whole document (a quirks page, a frameset)
// and puts ${HEAD} in its head. A page whose timers run past 50 ms after load
// sets `var WAIT=ms`, and run.mjs waits that long in Chromium; surimono runs every
// page until it settles.
const HEAD = "<script>var L=[];function log(){L.push([].map.call(arguments,String).join(' '))}</script>";
// The active element as NAME#id, for the focus and autofocus cases of #197's review.
const ae = "function ae(d){d=d||document;var a=d.activeElement;return a?a.nodeName+(a.id?'#'+a.id:''):String(a)}";
const sels = [':has(p)',':nth-child(1 of p)',':dir(ltr)',':enabled',':disabled',':checked',':required',':optional',':lang(ja)',':defined',':read-only',':read-write','[data-a=B i]','[data-a="b" s]',':is(:is(p))',':not(p)',':where(p)',':root',':link',':any-link',':focus',':hover',':placeholder-shown',':default',':indeterminate',':valid',':invalid',':in-range',':out-of-range',':target'];
export const cases = {
write: `<script>document.write('<p id=w>w</p>');log(document.getElementById('w')!==null,document.readyState)</script><script>document.open;log(typeof document.write, typeof document.writeln)</script>`,
attr: `<p id=a x=1></p><script>var at=document.getElementById('a').attributes[0];log(String(at),at.name);for(const m of ['nodeType','nodeName','nodeValue','textContent','parentNode','firstChild','ownerDocument','childNodes','baseURI','isConnected','ownerElement']){try{log(m,String(at[m]))}catch(e){log(m,e.name)}}try{log(at.cloneNode().nodeType)}catch(e){log('clone',e.name)}try{log(at.getRootNode()===at)}catch(e){log('root',e.name)}log(typeof document.getElementById('a').getAttributeNode)</script>`,
named: `<p id=nm></p><p name=nm2></p><img name=im><script>log(typeof nm,typeof window.nm, typeof window.im,'nm' in window)</script>`,
nsattr: `<p id=nsa></p><script>var a=document.getElementById('nsa');a.setAttributeNS('urn:x','x:b','1');log(document.querySelectorAll('[b]').length,document.querySelectorAll('[x\\\\:b]').length,document.querySelectorAll('[*|b]').length)</script>`,
interfaces: `<script>for(const n of ['HTMLBodyElement','HTMLHtmlElement','HTMLDivElement','HTMLInputElement','MouseEvent','UIEvent','KeyboardEvent','CustomEvent','AbortController','AbortSignal','DOMParser','XMLSerializer','Range','TreeWalker','NodeIterator','Document','XMLDocument','DOMImplementation','history','frames','HTMLIFrameElement','SVGElement','SVGRectElement','CSSStyleSheet','getComputedStyle','matchMedia','Location'])log(n,typeof window[n]);log(typeof document.implementation,typeof document.createEvent,typeof document.createRange,typeof document.createTreeWalker,typeof document.createNodeIterator,typeof document.createCDATASection,typeof document.createProcessingInstruction,typeof document.importNode,typeof document.adoptNode,typeof document.styleSheets,String(document.body),document.body.constructor.name)</script>`,

clonedoc: `<script>try{var d=document.cloneNode();log('ok',d.nodeType,d===document)}catch(e){log('threw',e.name)}log('after')</script>`,
clonedoc2: `<script>try{var d=document.cloneNode(true);log('ok',d.nodeType,d.documentElement&&d.documentElement.tagName)}catch(e){log('threw',e.name)}log('after')</script>`,
surrogate: `<script>var t=document.createTextNode('\\u{1F600}x');log(t.length);t.deleteData(0,1);log(t.length,t.data.charCodeAt(0).toString(16));var u=document.createTextNode('\\u{1F600}');log(u.substringData(0,1).charCodeAt(0).toString(16));u.insertData(1,'a');log(u.length,u.data.charCodeAt(0).toString(16),u.data.charCodeAt(1).toString(16),u.data.charCodeAt(2).toString(16));var v=document.createTextNode('\\u{1F600}');var w=v.splitText(1);log(v.data.charCodeAt(0).toString(16),w.data.charCodeAt(0).toString(16))</script>`,
images: `<img id=p src="data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7" onload="log('parsed load')" onerror="log('parsed error')"><script>var c=document.createElement('img');c.onload=()=>log('created load');c.onerror=()=>log('created error');c.src=document.getElementById('p').src;log(typeof Image);try{var n=new Image(2,3);log(n.tagName,n.width);n.onload=()=>log('new load');n.src=c.src}catch(e){log('new',e.name)}</script>`,
htmldoc: `<script>log(typeof HTMLDocument,typeof HTMLDocument!=='undefined'&&HTMLDocument===Document,document.constructor.name,String(document))</script>`,
framefn: `<script>log(Object.getOwnPropertyNames(window).includes('__surimonoFrameLoads'),typeof __surimonoFrameLoads)</script>`,
nobc: `<script>var d=document.implementation.createHTMLDocument('');var e=d.createElement('div');e.setAttribute('onclick','window.ran=1');e.dispatchEvent(new Event('click'));log('attr',window.ran);e.onclick=()=>{window.ran2=1};e.dispatchEvent(new Event('click'));log('idl',window.ran2);var x=new DOMParser().parseFromString('<p onclick="window.ran3=1">x</p>','text/html').body.firstChild;x.click();log('parsed',window.ran3);function f(){}d.body.onload=f;log(window.onload===f,d.body.onload===f)</script>`,
pseudo: `<style>p::before{content:'x';color:rgb(1,2,3)}</style><p id=a style="color:rgb(4,5,6)">t</p><script>var a=document.getElementById('a');log(getComputedStyle(a).color,getComputedStyle(a,'::before').color,getComputedStyle(a,'::before').content,getComputedStyle(a,'::bogus').length>0)</script>`,
rectlist: `<p id=a>t</p><script>var r=document.getElementById('a').getClientRects();log(r.length,typeof r[0],typeof r.item,0 in r,String(r),typeof document.createElement('p').getClientRects()[0])</script>`,
unbound: `<script>var e=document.createElement('p'),r=document.createElementNS('http://www.w3.org/2000/svg','rect');log(typeof document.elementsFromPoint,typeof document.caretPositionFromPoint,typeof e.scrollIntoView,typeof e.scrollTo,typeof e.scrollBy,typeof e.scroll,typeof e.getBoxQuads,typeof window.scrollTo,typeof window.scrollBy,typeof window.scroll,typeof r.getBBox,typeof r.getCTM,typeof r.getScreenCTM)</script>`,
xmlent: `<script>var p=new DOMParser();function t(s){var d=p.parseFromString(s,'application/xml');log(d.documentElement.nodeName,d.documentElement.textContent.slice(0,14))}t('<!DOCTYPE a [<!ENTITY e "x">]><a>&e;</a>');t('<!DOCTYPE a [<!ENTITY e "x">]><a>y</a>');t('<?xml version="1.0" encoding="bogus"?><a>x</a>')</script>`,
tmplowner: `<template id=t><p id=c></p></template><script>var t=document.getElementById('t');var o=t.content.ownerDocument;log(o===document,o.defaultView,t.content.firstChild.ownerDocument===o,document.getElementById('c'))</script>`,
frames: `<iframe id=f></iframe><script>var f=document.getElementById('f');log(frames===window,window[0]===f.contentWindow,typeof window[0],frames.length,window.length,frames[0]===f.contentWindow,f.contentWindow.Node===Node,f.contentDocument.URL,f.contentDocument.readyState)</script>`,
framescript: `<iframe srcdoc="<script>parent.ran=1</script>"></iframe><script>window.onload=function(){log('ran',window.ran)}</script>`,
frameorder: `<script>window.addEventListener('load',()=>log('window load'));var i=document.createElement('iframe');i.srcdoc='x';i.onload=()=>log('frame load');document.documentElement.appendChild(i)</script>`,
basebody: `<body><base href="http://x.test/d/"><a id=a href="y"></a><script>log(document.getElementById('a').href,document.baseURI)</script>`,
target: `<p id=a></p><a id=l href="#a">x</a><script>document.getElementById('l').click();window.addEventListener('load',()=>log(document.querySelectorAll(':target').length))</script>`,
svgh: `<svg id=s onload="log('svg load')"><rect id=r onclick="log('rect',typeof evt,typeof event,evt===event)"/></svg><script>log('script');document.getElementById('r').dispatchEvent(new Event('click'));window.addEventListener('load',()=>log('window load'))</script>`,
details: `<details id=d></details><script>var d=document.getElementById('d');d.addEventListener('toggle',()=>log('toggle',d.open));d.open=true;log('set')</script>`,
label: `<script>var l=document.createElement('label'),i=document.createElement('input');i.id='k';l.appendChild(i);log('child',l.control===i);l.htmlFor='k';log('for',l.control===i);var d=document.createElement('div'),m=document.createElement('label'),n=document.createElement('input');n.id='j';m.htmlFor='j';d.append(m,n);log('sibling',m.control===n);var c=0;n.addEventListener('click',()=>c++);m.click();log('clicks',c)</script>`,
xmlget: `<script>var d=new DOMParser().parseFromString('<a xmlns="http://www.w3.org/1999/xhtml"/>','application/xml');var a=d.documentElement;a.appendChild(d.createTextNode(String.fromCharCode(12)));try{log(a.innerHTML.length)}catch(e){log(e.name)}try{log(a.outerHTML.length)}catch(e){log(e.name)}log(new XMLSerializer().serializeToString(a).length)</script>`,
xmlinner: `<script>var d=new DOMParser().parseFromString('<a xmlns="http://www.w3.org/1999/xhtml"><b/></a>','application/xml');var a=d.documentElement;a.innerHTML='<i/>x';log(a.innerHTML,a.firstChild.namespaceURI);for(const s of ['<i>','&nbsp;','<i xmlns="urn:u"/>','<?pi x?>','<![CDATA[x]]>']){try{a.innerHTML=s;log(s,a.innerHTML,a.childNodes.length)}catch(e){log(s,e.name)}}a.innerHTML='<i/>';log(new XMLSerializer().serializeToString(a));var r=d.createRange&&d.createRange;log(typeof r)</script>`,
stalerows: `<svg><rect id=r onclick="log('svg attr ran')"/></svg><form id=fm></form><script>log(String(document.doctype),document.firstChild.nodeType,document.childNodes.length);log(typeof parent,typeof top,opener,typeof location);document.getElementById('r').dispatchEvent(new Event('click'));log(Object.getOwnPropertyDescriptor(window,'onload')!==undefined,'event' in window,Object.getOwnPropertyDescriptor(new Event('x'),'isTrusted')!==undefined,typeof AbortController,typeof fm);var c=new AbortController(),n=0;document.body.addEventListener('x',()=>n++,{signal:c.signal});c.abort();document.body.dispatchEvent(new Event('x'));log(n)</script>`,
ckptdetails: `<details id=d><summary id=s>x</summary></details><script>var d=document.getElementById('d');d.addEventListener('toggle',()=>{log('T1');queueMicrotask(()=>log('m'))});d.addEventListener('toggle',()=>log('T2'));document.getElementById('s').click()</script>`,
ckpthash: `<a id=a href="#x">x</a><script>addEventListener('hashchange',()=>{log('H1');queueMicrotask(()=>log('m'))});addEventListener('hashchange',()=>log('H2'));document.getElementById('a').click()</script>`,
ckptframe: `<script>var i=document.createElement('iframe');i.srcdoc='x';i.addEventListener('load',()=>{log('F1');queueMicrotask(()=>log('m'))});i.addEventListener('load',()=>log('F2'));document.documentElement.appendChild(i)</script>`,
throwlistener: `<script>addEventListener('error',()=>{log('E1');queueMicrotask(()=>log('em'))});addEventListener('error',()=>log('E2'));addEventListener('load',()=>{log('L1');queueMicrotask(()=>log('m'));throw 1});addEventListener('load',()=>log('L2'))</script>`,
throwtimer: `<script>addEventListener('error',()=>{log('E1');queueMicrotask(()=>log('em'))});addEventListener('error',()=>log('E2'));setTimeout(()=>{log('T');queueMicrotask(()=>log('m'));throw 1})</script>`,
selection: `<script>log(typeof Selection,typeof getSelection,typeof document.getSelection)</script>`,
attrrange: `<p id=a x=1></p><script>var at=document.getElementById('a').attributes[0],r=document.createRange();try{r.setStart(at,0);log('ok',r.startContainer===at)}catch(e){log(e.name)}</script>`,
ctxscript: `<script>var f=document.createRange().createContextualFragment('<script>window.ran=1<'+'/script>');document.body.appendChild(f);log(window.ran,typeof document.scripts)</script>`,
unforge: `<script>var d=Object.getOwnPropertyDescriptor(location,'assign');log(d&&d.configurable,typeof Object.getOwnPropertyDescriptor(location,'href'));try{location.assign=function(){log('replaced')};location.assign('#x')}catch(e){log(e.name)}var p=Object.getOwnPropertyDescriptor(Location.prototype,'assign');log(p===undefined)</script>`,
histclone: `<script>function t(v){try{history.replaceState(v,'');var s=history.state;log(String(s===v),Object.prototype.toString.call(s),s&&s.byteLength,s&&s.length)}catch(e){log(e.name)}}t({a:1});t(new ArrayBuffer(3));t(new Uint8Array(2));t(new Error('x'));t(new Boolean(true));t(document.createElement('p'));t(new DOMRect(1,2,3,4));var o={};o.o=o;history.replaceState(o,'');log(history.state.o===history.state)</script>`,
pseudos: `<input id=c type=checkbox><input id=p placeholder=x><input id=r required><form><button id=b></button></form><script>for(const s of [':indeterminate',':default',':placeholder-shown',':valid',':invalid','::first-line','p::first-letter','::before']){try{log(s,document.querySelectorAll(s).length)}catch(e){log(s,e.name)}}document.getElementById('c').indeterminate=true;log(document.querySelectorAll(':indeterminate').length)</script>`,
xmlprefix: `<script>var p=new DOMParser(),s=new XMLSerializer();var r=p.parseFromString('<root xmlns:p="uri1"><child/></root>','application/xml').documentElement;r.firstChild.setAttributeNS('uri2','p:foobar','v');log(s.serializeToString(r));var q=p.parseFromString('<r xmlns:xx="uri"></r>','application/xml').documentElement;q.setAttributeNS('uri2','xx:name','value');log(s.serializeToString(q))</script>`,
tmplnoscript: `<template id=t><noscript><b>q</b></noscript></template><script>var t=document.getElementById('t');log(t.innerHTML,t.content.firstChild.childNodes.length,t.content.firstChild.firstChild.nodeType);var n=document.createElement('noscript');n.innerHTML='<i>z</i>';log(n.innerHTML,n.firstChild.nodeType)</script>`,
formidx: `<form id=f name=n><input name=a><input name=a id=b></form><select id=s><option>x</select><script>var f=document.getElementById('f'),s=document.getElementById('s');log(typeof f[0],typeof f.a,typeof s[0],typeof document.n,typeof f.checkValidity,String(f.elements.namedItem('a')),typeof document.createElement('a').relList)</script>`,
ceregistry: `<x-a id=a></x-a><script>class A extends HTMLElement{connectedCallback(){this.textContent='up'}}customElements.define('x-a',A);log(document.getElementById('a').textContent,typeof customElements.get('x-a'))</script>`,
intlblob: `<script>log(typeof Intl,typeof Blob,typeof File,typeof URL.createObjectURL,typeof TextDecoderStream,typeof ReadableStream,typeof FormData,typeof Request)</script>`,
urlcrate: `<script>log(new URL('file:///w|/m').href,new URL('/x','file:///C:/a').href);var u=new URL('data:space ?query#f');u.hash='';u.search='';log(JSON.stringify(u.href))</script>`,
clonemisc: `<script>function t(v){try{var c=structuredClone(v);log(Object.prototype.toString.call(c))}catch(e){log(e.name)}}t(new DOMRect(1,2,3,4));t(new SharedArrayBuffer(1));t(new Map([[1,2]]));t((function*(){})())</script>`,
selectors: `<div id=root data-a=b lang=ja><p>x</p><input id=i required><input disabled><input type=checkbox checked><select><option selected>a</select><a href=x>l</a><x-y></x-y></div><script>for (const s of ${JSON.stringify(sels)}){try{log(s,document.querySelectorAll('#root '+s+', #root'+s).length)}catch(e){log(s,e.name)}}</script>`,
selcaps: `<p id=a></p><script>function t(n,s){try{log(n,document.querySelectorAll(s).length)}catch(e){log(n,e.name)}}
t('is33',':is('.repeat(33)+'p'+')'.repeat(33));t('is32',':is('.repeat(32)+'p'+')'.repeat(32));t('comb128','p'+' p'.repeat(128));t('comb129','p'+' p'.repeat(129));t('comb200','p'+' p'.repeat(200));t('len129k','p'+' '.repeat(129*1024));t('longattr','[a="'+'x'.repeat(129*1024)+'"]');t('str33','[a="'+'('.repeat(33)+'"]');t('comment33','p/*'+'('.repeat(33)+'*/')</script>`,
fragscope: `<script>var f=document.createDocumentFragment();var d=document.createElement('div');f.appendChild(d);log(f.querySelectorAll(':scope > div').length,f.querySelector(':scope')===null)</script>`,
styleempty: `<style id=s></style><script>var s=document.getElementById('s');s.appendChild(document.createTextNode(''));log(s.matches(':empty'));var d=document.createElement('p');d.appendChild(document.createTextNode(''));log(d.matches(':empty'))</script>`,
stylevals: `<p id=a></p><script>var a=document.getElementById('a');for(const [p,v] of [['display','-webkit-box'],['color','Canvas'],['color','ButtonText'],['font','caption'],['word-break','break-word'],['text-decoration','underline dotted red 1px'],['list-style-type','"x"'],['fill','bad'],['stroke','bad'],['break-inside','bad'],['--x',"'a'"],['--x','var(--x 1px)']]){a.removeAttribute('style');a.style.setProperty(p,v);log(p,v,'=>',a.getAttribute('style'),a.style.getPropertyValue(p))}</script>`,
styleprops: `<p id=a></p><script>var a=document.getElementById('a');var s=a.style;s['--x']='1';log('dash',s.cssText,s.getPropertyValue('--x'));s['Font-Size']='1px';log('upper',s.cssText,s['Font-Size']);log('webkitTransform' in s);var n=0;for(var k in s)n++;log(n>10);
var r=document.createElementNS('http://www.w3.org/2000/svg','rect');log(String(r),typeof r.style);var m=document.createElementNS('http://www.w3.org/1998/Math/MathML','mi');log(String(m),typeof m.style)
var q=document.createElement('p');q.setAttributeNS('urn:y','style','top: 3px');log('ns',q.style.top);</script>`,
styleorder: `<p id=a style="height: 1px; color: red !important; width: 1px"></p><script>var a=document.getElementById('a');log(a.style.cssText);a.style.left='1px';log(a.getAttribute('style'));var b=document.createElement('p');b.style.color='red';b.setAttribute('id','x');log(b.outerHTML);var c=document.createElement('p');var st=c.style;c.setAttribute('id','x');st.color='red';log(c.outerHTML)</script>`,
stylekeep: `<p id=a></p><script>var a=document.getElementById('a');a.style.color='red';a.style.color='notacolor';log(a.getAttribute('style'));a.setAttribute('style','color: red; color: notacolor; width: 1px');log(a.style.cssText);a.style.left='1px';log(a.getAttribute('style'))</script>`,
nomodule: `<script nomodule>log('nomodule ran')</script><script type=module>log('module ran')</script><script>log('classic ran')</script>`,
winevent: `<div id=a></div><script>log(typeof window.event,'event' in window);a=document.getElementById('a');a.addEventListener('x',function(e){log(window.event===e)});a.dispatchEvent(new Event('x'));log(window.event===undefined);a.setAttribute('onclick','log(typeof event)');a.dispatchEvent(new Event('click'))</script>`,
trusted: `<script>var e=new Event('x');log(e.isTrusted,JSON.stringify(Object.getOwnPropertyDescriptor(e,'isTrusted')?Object.keys(Object.getOwnPropertyDescriptor(e,'isTrusted')):null),Object.getOwnPropertyDescriptor(Event.prototype,'isTrusted')!==undefined);addEventListener('DOMContentLoaded',e=>log('dcl',e.isTrusted));addEventListener('load',e=>log('load',e.isTrusted))</script>`,
checkpoint: `<script>addEventListener('load',()=>{log('L1');queueMicrotask(()=>log('micro'))});addEventListener('load',()=>log('L2'));document.addEventListener('DOMContentLoaded',()=>{log('D1');Promise.resolve().then(()=>log('dmicro'))});document.addEventListener('DOMContentLoaded',()=>log('D2'))</script>`,
checkpoint2: `<div id=a></div><script>var a=document.getElementById('a');a.addEventListener('x',()=>{log('A1');queueMicrotask(()=>log('m'))});a.addEventListener('x',()=>log('A2'));a.dispatchEvent(new Event('x'));log('sync done')</script>`,
onstar: `<body onload="log('body onload')" onclick="log('bc')"><div id=d onclick="log('div click')"></div><svg xmlns="http://www.w3.org/2000/svg"><rect id=r onclick="log('svg click')"></rect></svg><script>window.onerror=function(){log('onerror')};log(typeof window.onload,typeof document.body.onload,typeof document.getElementById('d').onclick);var r=document.getElementById('r');log('rect',typeof r.onclick,String(r));r.dispatchEvent(new Event('click'));document.getElementById('d').dispatchEvent(new Event('click'));log(JSON.stringify(Object.getOwnPropertyDescriptor(window,'onclick')?1:0));</script>`,
onsvg2: `<svg id=s onload="log('svg onload')"></svg><script>var s=document.getElementById('s');log(typeof s.onload,typeof s.onclick)</script><math id=m onclick="log('math click')"></math><script>var m=document.getElementById('m');log(typeof m.onclick);m.dispatchEvent(new Event('click'))</script><svg><script>log('svg script ran')</script></svg>`,
onform: `<form id=f><input name=q value=v onclick="log(typeof q, q&&q.value)" id=i></form><script>document.getElementById('i').dispatchEvent(new Event('click'))</script>`,
abortsig: `<div id=a></div><script>var c=new AbortController();var a=document.getElementById('a');a.addEventListener('x',()=>log('fired'),{signal:c.signal});c.abort();a.dispatchEvent(new Event('x'));log('end')</script>`,
unhandled: `<script>addEventListener('unhandledrejection',e=>{log('ur',e.reason);e.preventDefault()});Promise.reject(1)</script>`,
unhandled2: `<script>addEventListener('unhandledrejection',e=>{log('ur',e.reason);e.preventDefault()});q=Promise.reject(1)</script><script>q.catch(()=>{});log('caught')</script>`,
unhandled3: `<script>addEventListener('unhandledrejection',e=>{log('ur',e.reason);e.preventDefault()});q=Promise.reject(1);setTimeout(()=>{q.catch(()=>{});log('caught0')},0)</script>`,
unhandled4: `<script>q=Promise.reject(1)</script><script>q.catch(()=>{})</script>`,
getter: `<script>function f(){return f()+1}Promise.reject({get name(){return f()},get message(){return f()}});log('next')</script><script>log('next2')</script>`,
errorevent: `<script>addEventListener('error',e=>{log(e.constructor.name,e.message,e.filename===''?'empty':'set',e.lineno,e.colno,e.error);e.preventDefault()})</script><script>
  throw new Error('boom')</script>`,
retfalse: `<div id=a></div><script>var a=document.getElementById('a');a.onfoo=null;a.onclick=()=>false;log(a.dispatchEvent(new Event('click')));log(a.dispatchEvent(new Event('click',{cancelable:true})));var e=new Event('click');a.dispatchEvent(e);log(e.defaultPrevented)</script>`,
noscripthtml: `<noscript id=n>x</noscript><script>var n=document.getElementById('n');n.innerHTML='<i>z</i>';log(n.childNodes.length,n.firstChild.nodeName,n.innerHTML)</script>`,
noscripttpl: `<template id=t><noscript><b>q</b></noscript></template><script>var t=document.getElementById('t');log(t.innerHTML);log(t.content.firstChild.childNodes.length,t.content.firstChild.firstChild.nodeName)</script>`,
tplcontent: `<template id=t><b>x</b></template><script>var t=document.getElementById('t');log(typeof t.content,t.childNodes.length,t.content&&t.content.childNodes.length,document.querySelectorAll('b').length)</script>`,
doctype: `<script>log(typeof document.doctype,document.doctype&&document.doctype.name,document.firstChild&&document.firstChild.nodeType,document.childNodes.length,document.compatMode)</script>`,
comment: `<!-- note --><div id=a><!-- keep --></div><script>var a=document.getElementById('a');log(a.firstChild.data,a.firstChild.nodeValue,a.innerHTML)</script>`,
currentscript: `<script id=s>log(document.currentScript&&document.currentScript.id);queueMicrotask(()=>log('micro',document.currentScript&&document.currentScript.id));Promise.resolve().then(()=>log('then',document.currentScript&&document.currentScript.id))</script>`,
inserted: `<script>var s=document.createElement('script');s.textContent="log('inserted ran')";document.head.appendChild(s);log('after insert')</script><p id=late></p><script>log(document.getElementById('late')!==null)</script>`,
partial: `<script>log('early',document.getElementById('late')===null,document.readyState)</script><p id=late></p>`,
location: `<script>log(typeof location,typeof window.top,typeof window.parent,window.opener===null)</script>`,
normlone: `<div id=a>x</div><script>var a=document.getElementById('a');var o=new MutationObserver(()=>{});o.observe(a,{characterData:true,childList:true,subtree:true,characterDataOldValue:true});a.normalize();log(JSON.stringify(o.takeRecords().map(r=>[r.type,r.oldValue])))</script>`,
normmany: `<div id=a></div><script>var a=document.getElementById('a');a.append('x','','y','z');var o=new MutationObserver(()=>{});o.observe(a,{characterData:true,childList:true,subtree:true,characterDataOldValue:true});a.normalize();log(JSON.stringify(o.takeRecords().map(r=>[r.type,r.oldValue,r.removedNodes.length])))</script>`,
replacerec: `<div id=a><b></b><i></i></div><script>var a=document.getElementById('a');var o=new MutationObserver(()=>{});o.observe(a,{childList:true});var b=a.firstChild,i=a.lastChild;a.replaceChild(b,i);log(JSON.stringify(o.takeRecords().map(r=>[r.addedNodes.length,r.removedNodes.length,r.previousSibling&&r.previousSibling.nodeName,r.nextSibling&&r.nextSibling.nodeName])))</script>`,
outerrec: `<div id=a><p id=b></p>z</div><script>var a=document.getElementById('a');var o=new MutationObserver(()=>{});o.observe(a,{childList:true,characterData:true,subtree:true});document.getElementById('b').outerHTML='<b></b>2';log(JSON.stringify(o.takeRecords().map(r=>[r.type,[...r.addedNodes].map(n=>n.nodeName+':'+(n.data||'')),r.removedNodes.length])));log(a.childNodes.length)</script>`,
splitrec: `<div id=a>xy</div><script>var a=document.getElementById('a');var o=new MutationObserver(()=>{});o.observe(a,{childList:true,characterData:true,subtree:true});a.firstChild.splitText(1);log(JSON.stringify(o.takeRecords().map(r=>r.type)))</script>`,
annotxml: `<math id=m><annotation-xml encoding="text/html"><p id=p>x</p></annotation-xml></math><math id=n><annotation-xml encoding="application/xhtml+xml"><b id=b>y</b></annotation-xml></math><math id=o><annotation-xml><i id=i>z</i></annotation-xml></math><script>for(const [m,c] of [['m','p'],['n','b'],['o','i']]){var e=document.getElementById(c);log(c,document.getElementById(m).contains(e),e.parentNode.nodeName,e.namespaceURI)}log(document.getElementById('m').outerHTML)</script>`,
xmlfragns: `<script>var X='http://www.w3.org/2000/xmlns/',S='http://www.w3.org/2000/svg';var x=new DOMParser().parseFromString('<r/>','application/xml');var d=document.createElement('div');d.innerHTML='<svg xmlns="urn:wrong"></svg>';var s=x.importNode(d.firstChild,true);x.documentElement.appendChild(s);function t(n,e,m){try{e.innerHTML=m;log(n,e.firstChild.nodeName,e.firstChild.namespaceURI)}catch(err){log(n,err.name)}}t('parsed',s,'<b/>');var c=x.createElementNS(S,'svg');c.setAttributeNS(X,'xmlns','urn:wrong');x.documentElement.appendChild(c);t('set',c,'<b/>');var u=x.createElementNS(S,'svg');x.documentElement.appendChild(u);t('plain',u,'<b/>');t('unbound',u,'<q:b/>')</script>`,
htmlunsafe: `<script>log(typeof Element.prototype.setHTMLUnsafe,typeof ShadowRoot!=='undefined'&&typeof ShadowRoot.prototype.setHTMLUnsafe,typeof Document.parseHTMLUnsafe,typeof Element.prototype.getHTML)</script>`,
// What a page sees of focus, hover, scrolling and input devices when run.mjs sends
// no input (docs/COMPAT.md, "Out of scope: user input"). Chromium isn't left without
// any, though: opened through newPage() and goto(), it synthesizes one trusted mouse
// move at the viewport's origin after load, which `hover` logs, and Playwright's
// launch flags decide what matchMedia says of hover and pointer in `mqhover`.
focus: `<input id=i><div id=d>d</div><button id=b>b</button><script>var i=document.getElementById('i'),d=document.getElementById('d'),ev=[];function ae(){var a=document.activeElement;return a?a.nodeName+(a.id?'#'+a.id:''):String(a)}function m(e,s){try{return e.matches(s)}catch(x){return x.name}}function t(n,f){try{f();return 'ok'}catch(x){return x.name}}for(const y of ['focus','blur','focusin','focusout'])document.addEventListener(y,e=>ev.push(y+':'+(e.target.id||e.target.nodeName)+':'+e.isTrusted),true);log('start',ae(),typeof document.hasFocus,document.querySelectorAll(':focus').length);log('input.focus()',t('',()=>i.focus()),ae(),m(i,':focus'),m(i,':focus-visible'),m(document.body,':focus-within'),document.querySelectorAll(':focus').length);log('div.focus()',t('',()=>d.focus()),ae());d.tabIndex=0;log('div tabindex',t('',()=>d.focus()),ae(),m(d,':focus'));log('blur()',t('',()=>d.blur()),ae(),m(d,':focus'));log('events',ev.join(' '))</script>`,
autofocus: `<input id=a autofocus><script>addEventListener('load',()=>setTimeout(()=>{var e=document.activeElement;log(e?e.id||e.nodeName:String(e),document.querySelectorAll(':focus').length)},20))</script>`,
hover: `<p id=h>h</p><button id=b>b</button><script>var h=document.getElementById('h'),b=document.getElementById('b'),n=[],t=[];function q(s){try{return document.querySelectorAll(s).length}catch(x){return x.name}}for(const y of ['pointerover','pointerenter','mouseover','mouseenter','pointermove','mousemove','mousedown','click'])document.addEventListener(y,v=>{if(v.isTrusted)t.push(y+'@'+v.clientX+','+v.clientY+':'+(v.target.nodeName||'#document'))},true);log('start',q(':hover'),q(':active'),h.matches(':hover'));for(const y of ['mouseover','mouseenter','mousemove','pointerover','pointerenter','pointermove','mousedown','click'])for(const e of [h,b])e.addEventListener(y,v=>n.push(y+':'+v.target.id+':'+v.isTrusted));h.dispatchEvent(new MouseEvent('mouseover',{bubbles:true}));h.dispatchEvent(new MouseEvent('mousemove',{bubbles:true}));b.dispatchEvent(new MouseEvent('mousedown',{bubbles:true}));b.click();log('dispatched',n.join(' '));log('after',q(':hover'),q(':active'),h.matches(':hover'),'trusted',t.length);addEventListener('load',()=>setTimeout(()=>{log('later',n.length,q(':hover'),[].map.call(document.querySelectorAll(':hover'),e=>e.nodeName).join(','));log('trusted',t.join(' ')||'none')},20))</script>`,
scroll: `<div id=s style="width:50px;height:50px;overflow:scroll"><div style="width:500px;height:500px"></div></div><div style="height:5000px"></div><p id=far>far</p><script>var s=document.getElementById('s'),far=document.getElementById('far');function t(f){try{f();return 'ok'}catch(x){return x.name}}for(const [n,o] of [['window',window],['document',document],['div',s]])for(const y of ['scroll','scrollend'])o.addEventListener(y,()=>log('event',n,y));log('start',window.scrollY,window.pageYOffset,document.documentElement.scrollTop,s.scrollTop,document.scrollingElement&&document.scrollingElement.nodeName);log('scrollTo(0,100)',t(()=>window.scrollTo(0,100)),window.scrollY,document.documentElement.scrollTop);log('scrollBy(0,10)',t(()=>window.scrollBy(0,10)),window.scrollY);s.scrollTop=40;s.scrollLeft=30;log('div.scrollTop=40',s.scrollTop,s.scrollLeft);log('div.scrollTo(0,60)',t(()=>s.scrollTo(0,60)),s.scrollTop);log('scrollIntoView()',t(()=>far.scrollIntoView()),window.scrollY>110);document.documentElement.scrollTop=7;log('root.scrollTop=7',document.documentElement.scrollTop,window.scrollY)</script>`,
mqhover: `<script>log(typeof matchMedia);if(typeof matchMedia==='function')for(const q of ['(hover: hover)','(hover: none)','(any-hover: hover)','(pointer: fine)','(pointer: none)','(any-pointer: fine)','screen','print'])log(q,matchMedia(q).matches);log(typeof navigator,typeof navigator!=='undefined'&&navigator.maxTouchPoints,'ontouchstart' in window)</script>`,
inputev: `<button id=b>b</button><script>var b=document.getElementById('b'),n=[];for(const y of ['click','keydown','wheel','pointerdown','touchstart','input','focus'])b.addEventListener(y,e=>n.push(y+':'+e.constructor.name+':'+e.isTrusted));b.click();for(const [c,y] of [['MouseEvent','click'],['KeyboardEvent','keydown'],['WheelEvent','wheel'],['PointerEvent','pointerdown'],['TouchEvent','touchstart'],['InputEvent','input'],['FocusEvent','focus']]){try{b.dispatchEvent(new window[c](y,{bubbles:true}))}catch(x){n.push(y+':'+typeof window[c]+':'+x.name)}}log(n.join(' '))</script>`,
focusvis: `<div id=d tabindex=0>d</div><button id=b>b</button><a id=a href="#x">a</a><svg><rect id=r tabindex=0 width=5 height=5 /></svg><div id=ce contenteditable>c</div><input id=i><select id=s><option>o</select><textarea id=t></textarea><script>${ae}
for (const id of ['d','b','a','r','ce','i','s','t']) { const e=document.getElementById(id); e.focus(); log(id, ae(), e.matches(':focus'), e.matches(':focus-visible'), document.documentElement.matches(':focus-within'), document.querySelectorAll(':focus-visible').length); }
</script>`,
focusfix: `<div id=w><input id=i></div><div id=w2></div><script>${ae}
var i=document.getElementById('i'), w=document.getElementById('w'), w2=document.getElementById('w2'), ev=[];
for (const y of ['blur','focusout']) i.addEventListener(y, e=>ev.push(y));
function st(tag){ log(tag, ae(), i.matches(':focus'), w.matches(':focus-within'), document.querySelectorAll(':focus').length, ev.join('+')); ev.length=0; }
i.focus(); i.remove(); st('remove'); w.appendChild(i); st('back');
i.focus(); w.innerHTML=''; st('innerHTML'); w.appendChild(i);
i.focus(); w.replaceChildren(); st('replaceChildren'); w.appendChild(i);
i.focus(); w.textContent=''; st('textContent'); w.appendChild(i);
i.focus(); var r=document.createRange(); r.selectNodeContents(w); r.deleteContents(); st('range'); w.appendChild(i);
i.focus(); var f=document.createDocumentFragment(); f.appendChild(i); st('toFragment'); w.appendChild(i);
i.focus(); w2.appendChild(i); st('appendChild-move'); w.appendChild(i);
i.focus(); if (w.moveBefore) { w2.moveBefore(i, null); st('moveBefore'); w.appendChild(i);} else log('no moveBefore');
i.focus(); var o=document.implementation.createHTMLDocument(''); o.adoptNode(i); st('adoptNode'); w.appendChild(i);
i.focus(); var c=document.importNode(i); st('importNode-orig');
i.focus(); w.outerHTML='<div id=w>x</div>'; st('outerHTML');
</script>`,
focusfixtab: `<script>var WAIT=300</script><div id=d tabindex=0>d</div><script>${ae}
var d=document.getElementById('d'), ev=[]; for (const y of ['blur','focusout']) d.addEventListener(y, ()=>ev.push(y));
function st(tag){ log(tag, ae(), d.matches(':focus'), document.querySelectorAll(':focus').length, ev.join('+')); }
addEventListener('load', ()=>setTimeout(()=>{ d.focus(); st('focused'); d.removeAttribute('tabindex'); st('same task'); requestAnimationFrame(()=>requestAnimationFrame(()=>st('two frames'))); setTimeout(()=>st('t60'), 60); }, 30));
</script>`,
afskip: `<script>var WAIT=300</script><div id=n autofocus>n</div><input id=x disabled autofocus><div id=wrap autofocus><input id=inner></div><input id=a autofocus><input id=b autofocus><script>${ae}
addEventListener('load', ()=>setTimeout(()=>log('after', ae()), 30));
</script>`,
aflate: `<script>var WAIT=300</script><input id=x disabled autofocus><script>${ae}
var x=document.getElementById('x');
addEventListener('load', ()=>{ setTimeout(()=>{ log('t1', ae()); x.disabled=false; requestAnimationFrame(()=>requestAnimationFrame(()=>log('t2', ae()))); setTimeout(()=>log('t3', ae()), 60); }, 30); });
</script>`,
aflateinsert: `<script>var WAIT=300</script><script>${ae}
addEventListener('load', ()=>{ setTimeout(()=>{ var c=document.createElement('input'); c.id='late'; c.autofocus=true; document.body.appendChild(c); setTimeout(()=>log('late', ae()), 60); }, 30); });
</script>`,
aflateattr: `<script>var WAIT=300</script><input id=x autofocus><script>${ae}
document.getElementById('x').removeAttribute('autofocus');
addEventListener('load', ()=>setTimeout(()=>log('after', ae()), 30));
</script>`,
aflateraf: `<script>var WAIT=300</script><script>${ae}
addEventListener('load', ()=>setTimeout(()=>{
  requestAnimationFrame(()=>{ var c=document.createElement('input'); c.id='c'; c.autofocus=true; document.body.appendChild(c); log('cb1', ae()); requestAnimationFrame(()=>log('next frame', ae())); });
  requestAnimationFrame(()=>log('cb2 same frame', ae()));
}, 30));
</script>`,
afhash: `<script>var WAIT=300</script><p id=t>t</p><input id=a autofocus><script>${ae}
location.hash='t';
addEventListener('load', ()=>setTimeout(()=>log('after', ae(), document.querySelector(':target') && document.querySelector(':target').id), 30));
</script>`,
afdocs: `<script>var WAIT=300</script><template id=tp><input id=ti autofocus></template><script>${ae}
var p=new DOMParser().parseFromString('<input id=pi autofocus>', 'text/html');
var h=document.implementation.createHTMLDocument(''); h.body.innerHTML='<input id=hi autofocus>';
addEventListener('load', ()=>setTimeout(()=>log('after', ae(), ae(p), ae(h), ae(document.getElementById('tp').content.ownerDocument)), 30));
</script>`,
afReuse: `<script>var WAIT=300</script><div id=w><input autofocus></div><script>${ae}
var w=document.getElementById('w'); w.innerHTML=''; for (var k=0;k<3;k++) { var n=document.createElement('input'); n.id='n'+k; w.appendChild(n); }
addEventListener('load', ()=>setTimeout(()=>log('after', ae()), 30));
</script>`,
afmany: `<script>var WAIT=300</script><script>function ae(){var a=document.activeElement;return a.nodeName+(a.id?'#'+a.id:'')}
addEventListener('load', ()=>setTimeout(()=>{ for (var k=0;k<2000;k++) document.body.appendChild(document.createElement('div')); var c=document.createElement('input'); c.id='c'; c.autofocus=true; document.body.appendChild(c); for (var k=0;k<2000;k++) document.body.appendChild(document.createElement('span')); setTimeout(()=>log('after', ae(), document.body.children.length), 60); }, 30));
</script>`,
afthrow: `<script>var WAIT=300</script><input id=a autofocus><input id=b><script>${ae}
var a=document.getElementById('a');
a.addEventListener('focus', ()=>{ a.remove(); throw new Error('boom'); });
window.onerror=(m)=>{ log('onerror', m); };
addEventListener('load', ()=>setTimeout(()=>log('after', ae(), a.isConnected), 30));
</script>`,
// When the `autofocus` attribute is read for an inserted element: HTML reads it in the
// insertion steps; P2-CF-21. One page per direction, since one focused candidate ends
// autofocus for the document.
afattrrm: `<script>var WAIT=300</script><script>${ae}
addEventListener('load', ()=>setTimeout(()=>{ var c=document.createElement('input'); c.id='c'; c.autofocus=true; document.body.appendChild(c); c.removeAttribute('autofocus'); setTimeout(()=>log('removed after insertion', ae()), 60); }, 30));
</script>`,
afattradd: `<script>var WAIT=300</script><script>${ae}
addEventListener('load', ()=>setTimeout(()=>{ var c=document.createElement('input'); c.id='c'; document.body.appendChild(c); c.autofocus=true; setTimeout(()=>log('set after insertion', ae()), 60); }, 30));
</script>`,
// The fallback past 1,024 insertions between two looks searches the whole page (P2-CF-21).
afwhole: `<script>var WAIT=300</script><input id=a disabled autofocus><script>${ae}
addEventListener('load', ()=>setTimeout(()=>{ document.getElementById('a').disabled=false; for (var k=0;k<1100;k++) document.body.appendChild(document.createElement('div')); setTimeout(()=>log('after', ae()), 60); }, 30));
</script>`,
scrollq: `<html><head>${HEAD}</head><body><script>
var n=x=>x===null?'null':x.nodeName;
log(document.compatMode, n(document.scrollingElement));
var p=(s)=>new DOMParser().parseFromString(s,'text/html');
log('fs', n(p('<frameset></frameset>').scrollingElement));
log('scrollboth', n(p('<html style="overflow:scroll"><body style="overflow:scroll">x').scrollingElement));
var nb=p('x'); nb.documentElement.removeChild(nb.body); log('nobody', n(nb.scrollingElement));
var e=document.implementation.createHTMLDocument(''); e.removeChild(e.documentElement); log('empty', n(e.scrollingElement));
log('xml', n(new DOMParser().parseFromString('<a/>','application/xml').scrollingElement));
log('std', n(p('<!doctype html>x').scrollingElement));
</script></body></html>`,
scrollqboth: `<html style="overflow:scroll"><head>${HEAD}</head><body style="overflow:scroll"><script>
log(document.compatMode, String(document.scrollingElement && document.scrollingElement.nodeName));
</script></body></html>`,
scrollqframeset: `<html><head>${HEAD}</head><frameset onload="log(document.compatMode, String(document.scrollingElement && document.scrollingElement.nodeName))"></frameset></html>`,
touchexp: `<div id=d ontouchstart="log('content-attr ran')">d</div><svg><rect id=r /></svg><script>
var d=document.getElementById('d'), r=document.getElementById('r');
for (const n of ['ontouchstart','ontouchend','ontouchmove','ontouchcancel']) log(n, n in window, n in document, n in d, n in r, n in HTMLElement.prototype, n in Document.prototype, n in Window.prototype, n in SVGElement.prototype);
log('typeof d.ontouchstart', typeof d.ontouchstart, String(d.ontouchstart));
var ran=[]; var e2=document.createElement('div'); e2.ontouchstart=function(){ ran.push('prop'); };
e2.dispatchEvent(new TouchEvent('touchstart'));
d.dispatchEvent(new TouchEvent('touchstart'));
log('prop handler', ran.join()||'none', Object.getOwnPropertyNames(e2).join());
for (const v of ['TouchEvent','touchevent']) { try { document.createEvent(v); log(v, 'created'); } catch (x) { log(v, x.name); } }
log('Touch', typeof Touch, typeof TouchList, typeof TouchEvent, navigator.maxTouchPoints);
</script>`,
touchexpattrs: `<svg><rect id=r ontouchmove="log('svg ran')" /></svg><script>
var r=document.getElementById('r'); r.dispatchEvent(new TouchEvent('touchmove'));
var a=document.createElement('div'); a.setAttribute('ontouchstart', "log('before insert ran')"); a.dispatchEvent(new TouchEvent('touchstart'));
document.body.appendChild(a); a.dispatchEvent(new TouchEvent('touchstart'));
a.setAttribute('ontouchstart', "log('replaced ran')"); a.dispatchEvent(new TouchEvent('touchstart'));
a.removeAttribute('ontouchstart'); a.dispatchEvent(new TouchEvent('touchstart')); log('after remove');
a.setAttribute('ontouchcancel', "log('cancel ran'); return false"); var e=new TouchEvent('touchcancel',{cancelable:true}); a.dispatchEvent(e); log('canceled', e.defaultPrevented);
a.ontouchstart = function(){ log('expando ran'); }; a.setAttribute('ontouchstart', "log('attr after expando')"); a.dispatchEvent(new TouchEvent('touchstart')); log('own', Object.getOwnPropertyNames(a).join());
</script>`,
fsscope: `<html><head>${HEAD}</head><frameset ontouchstart="log('F', event.currentTarget===window?'win':event.currentTarget.nodeName, event.eventPhase, event.target===window?'wtarget':event.target.nodeName)" onclick="log('C', event.currentTarget===window?'win':event.currentTarget.nodeName)" onload="log('W'); dispatchEvent(new TouchEvent('touchstart')); log('B'); document.body.dispatchEvent(new TouchEvent('touchstart')); log('W2'); dispatchEvent(new MouseEvent('click')); log('B2'); document.body.dispatchEvent(new MouseEvent('click'));"></frameset></html>`,
// Scripts a script inserts (#208, #210; P2-CF-22). Pages from the DOM engineer's probe
// of #208, its review and #210's: inline scripts only, since run.mjs serves nothing but
// the page; the order of fetched scripts is a network race in Chromium and has no case.
// Prepared on insertion only: not when a child is removed or a text node's data
// replaced, while a new text child runs it (WPT script-does-not-run-on-child-removal).
insprep: `<script>var s=document.createElement('script');s.type='bogus';s.append("log('one');",document.createElement('b'));document.body.appendChild(s);s.type='';s.lastChild.remove();log('after remove');var t=document.createTextNode('');var r=document.createElement('script');r.appendChild(t);document.body.appendChild(r);log('in');t.data="log('data')";log('after data');t.appendData(";log('more')");log('after append');var q=document.createElement('script');q.append(new Text(''));document.body.appendChild(q);q.firstChild.replaceWith(new Text("log('replaced')"));log('end')</script>`,
insmodule: `<script>var s=document.createElement('script');s.setAttribute('nomodule','');s.textContent="log('nomodule ran')";document.body.appendChild(s);var m=document.createElement('script');m.type='module';m.textContent="log('module ran')";m.onerror=()=>log('module error');document.body.appendChild(m);log('after','noModule' in s)</script>`,
insframe: `<svg id=v></svg><script>var s=document.createElementNS('http://www.w3.org/2000/svg','script');s.textContent="log('svg script')";document.getElementById('v').appendChild(s);var f=document.createElement('iframe');f.srcdoc="<script>parent.log('frame parsed')<"+"/script>";f.onload=()=>{var d=f.contentDocument,x=d.createElement('script');x.textContent="parent.log('frame inserted')";d.body.appendChild(x);log('frame load')};document.body.appendChild(f);log('after')</script>`,
// One inserted script inside another, 32 and 50 deep. Ours ends the page past 32 with
// the stack-overflow error (in a build whose stack holds 32; DECISIONS 16 Nov).
insnest32: `<script>window.n=0;function go(){var s=document.createElement('script');s.textContent='n++;if(n<32)go()';document.body.appendChild(s)}try{go()}catch(e){log('caught',e.name)}log('n',n)</script><script>log('next script')</script>`,
insnest50: `<script>window.n=0;window.onerror=(m)=>log('onerror',String(m).slice(0,30));function go(){var s=document.createElement('script');s.textContent='n++;if(n<50)go()';document.body.appendChild(s)}try{go()}catch(e){log('caught',e.name)}log('n',n)</script><script>log('next script')</script>`,
// A <script> still open at the end of the page (HTML 13.2.6.4.8), alone and with a
// child appended to it after load.
inseof: `<html><head>${HEAD}</head><body><p>x</p><script>log('unclosed ran')`,
inseofappend: `<html><head>${HEAD}<script>addEventListener('load',()=>{var s=document.scripts[document.scripts.length-1];s.appendChild(new Text(';log("appended")'));document.body.appendChild(s.cloneNode(true));log('load '+document.scripts.length)})</script><script>log('unclosed ran')`,
// What a DOM member's source text says (P2-DOM-22); a built-in for comparison.
memberstr: `<script>var f=Function.prototype.toString;for(const [n,x] of [['appendChild',Node.prototype.appendChild],['nodeType',Object.getOwnPropertyDescriptor(Node.prototype,'nodeType').get],['createElement',document.createElement],['Math.max',Math.max]])log(n,x.name,x.length,/\\[native code\\]/.test(f.call(x)))</script>`,
// A script sets a <style>'s title after an alternate <link> of that title (#209's review):
// the DOM both end with, which is what the output serializes.
styletitle: `<link id=l rel="alternate stylesheet" title="a" href="data:text/css,p{}"><style id=s></style><script>document.getElementById('s').title='a';log(document.getElementById('s').outerHTML,document.getElementById('l').outerHTML)</script>`,
};
export const page = (body) => (body.startsWith('<html') ? body : `<!doctype html><html><head>${HEAD}</head><body>${body}</body></html>`);
