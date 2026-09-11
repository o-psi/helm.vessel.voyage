// Package sdk implements the private Voyage extension protocol version 1.
// It imports only Go's standard library, not Voyage internals. Stdout is private
// protocol traffic; errors are authored codes, never arbitrary diagnostics.
package sdk

import (
    "bufio"
    "bytes"
    "context"
    "encoding/json"
    "errors"
    "io"
    "os"
    "reflect"
    "sync"
    "time"
    "unicode/utf8"
)

const MaxFrame = 1024 * 1024
var ErrProtocol = errors.New("extension protocol failure")
var ErrDenied = errors.New("host request denied")

type Identity struct {
    Session string `json:"session"`
    Incarnation string `json:"incarnation"`
    Run string `json:"run"`
    Invocation string `json:"invocation"`
    Package string `json:"package"`
    Digest string `json:"digest"`
}
type message struct {
    Type string `json:"type"`
    Protocol int `json:"protocol,omitempty"`
    Identity *Identity `json:"identity,omitempty"`
    Definitions json.RawMessage `json:"definitions,omitempty"`
    Capabilities []string `json:"capabilities,omitempty"`
    Invocation string `json:"invocation,omitempty"`
    Kind string `json:"kind,omitempty"`
    Name string `json:"name,omitempty"`
    Arguments json.RawMessage `json:"arguments,omitempty"`
    Request uint32 `json:"request,omitempty"`
    Text string `json:"text,omitempty"`
    Code string `json:"code,omitempty"`
}
type Handler func(context.Context, *Call, string, string, json.RawMessage) (any, error)
type Call struct {
    identity Identity
    out *output
    replies chan message
    readGate chan struct{}
    mu sync.Mutex
    progress []time.Time
    progressCount int
    requests uint32
    canRead bool
    closed bool
}
type output struct { mu sync.Mutex; writer io.Writer }
func (o *output) send(v any) error {
    bytes, err := json.Marshal(v)
    if err != nil || len(bytes) > MaxFrame { return ErrProtocol }
    if _, err = strict(bytes); err != nil { return err }
    o.mu.Lock(); defer o.mu.Unlock()
    _, err = o.writer.Write(append(bytes, '\n'))
    return err
}
func (c *Call) Progress(text string) error {
    c.mu.Lock(); defer c.mu.Unlock()
    if c.closed || len(c.readGate)>0 || !utf8.ValidString(text) || len(text)>4096 || c.progressCount>=256 { return ErrProtocol }
    now:=time.Now()
    for len(c.progress)>0 && now.Sub(c.progress[0])>=time.Second { c.progress=c.progress[1:] }
    if len(c.progress)>=10 { return ErrProtocol }
    c.progress=append(c.progress,now); c.progressCount++
    return c.out.send(map[string]any{"type":"progress","invocation":c.identity.Invocation,"text":text})
}
// Read uses only the host broker. There is deliberately no local file fallback.
// The host may refuse roots, access, approval, UTF-8, or redaction constraints.
func (c *Call) Read(ctx context.Context, path string, offset uint64, maxBytes int) (string,error) {
    if maxBytes<1 || maxBytes>65536 || len(path)<1 || len(path)>4096 { return "",ErrProtocol }
    select { case c.readGate<-struct{}{}: default: return "",ErrProtocol }
    defer func(){ <-c.readGate }()
    c.mu.Lock()
    if c.closed || !c.canRead || c.requests>=32 { c.mu.Unlock(); return "",ErrDenied }
    c.requests++; request:=c.requests
    err:=c.out.send(map[string]any{"type":"host.file.read","invocation":c.identity.Invocation,
        "request":request,"path":path,"offset":offset,"max_bytes":maxBytes})
    c.mu.Unlock()
    if err!=nil { return "",err }
    select {
    case <-ctx.Done(): return "",ctx.Err()
    case reply:=<-c.replies:
        if reply.Request!=request || reply.Invocation!=c.identity.Invocation { return "",ErrProtocol }
        if reply.Type=="host_error" { return "",ErrDenied }
        if reply.Type!="host_result" || len(reply.Text)>maxBytes { return "",ErrProtocol }
        return reply.Text,nil
    }
}

// Serve handles one admitted invocation per process. The host owns deadlines,
// isolation and cleanup. Handlers must cooperate with ctx cancellation; ignoring
// it cannot prevent the host terminating the isolated process.
func Serve(definitions json.RawMessage, capabilities []string, handler Handler) error {
    return serve(os.Stdin, os.Stdout, definitions, capabilities, handler)
}
func serve(reader io.Reader, writer io.Writer, definitions json.RawMessage, capabilities []string, handler Handler) error {
    pinned,err:=strict(definitions); if err!=nil { return err }
    out:=&output{writer:writer}
    frames:=make(chan message,1); failed:=make(chan error,1)
    go func(){
        r:=bufio.NewReaderSize(reader,4096)
        for {
            frame,err:=readFrame(r)
            if err!=nil { failed<-err; return }
            if !validFields(frame) { failed<-ErrProtocol; return }
            var m message
            d:=json.NewDecoder(bytes.NewReader(frame)); d.DisallowUnknownFields()
            if err=d.Decode(&m); err!=nil { failed<-ErrProtocol; return }
            frames<-m
        }
    }()
    receive:=func()(message,error){ select { case m:=<-frames:return m,nil; case err:=<-failed:return message{},err } }
    init,err:=receive(); if err!=nil { return err }
    got,err:=strict(init.Definitions)
    if err!=nil || init.Type!="initialize" || init.Protocol!=1 || init.Identity==nil ||
        !reflect.DeepEqual(got,pinned) || !reflect.DeepEqual(init.Capabilities,capabilities) { return ErrProtocol }
    if err=out.send(map[string]any{"type":"initialized","protocol":1,"identity":init.Identity,
        "definitions":definitions,"capabilities":capabilities}); err!=nil { return err }
    invoke,err:=receive(); if err!=nil { return err }
    if invoke.Type=="shutdown" { return out.send(map[string]any{"type":"shutdown_ack"}) }
    if invoke.Type!="invoke" || invoke.Invocation!=init.Identity.Invocation ||
        (invoke.Kind!="tool" && invoke.Kind!="command" && invoke.Kind!="lifecycle") { return ErrProtocol }
    ctx,cancel:=context.WithCancel(context.Background()); defer cancel()
    c:=&Call{identity:*init.Identity,out:out,replies:make(chan message,1),readGate:make(chan struct{},1)}
    for _,capability:=range capabilities { if capability=="host.file.read" { c.canRead=true } }
    type result struct { value any; err error }
    done:=make(chan result,1)
    go func(){ v,e:=handler(ctx,c,invoke.Kind,invoke.Name,invoke.Arguments); done<-result{v,e} }()
    finished:=false
    for {
        select {
        case err:=<-failed: return err
        case r:=<-done:
            c.mu.Lock(); c.closed=true; c.mu.Unlock()
            if !finished {
                var reply any=map[string]any{"type":"result","invocation":c.identity.Invocation,"value":r.value}
                if r.err!=nil { reply=map[string]any{"type":"error","invocation":c.identity.Invocation,"code":"failed"} }
                if err:=out.send(reply); err!=nil { return err }
                finished=true
            }
        case m:=<-frames:
            switch m.Type {
            case "cancel":
                if m.Invocation!=c.identity.Invocation { return ErrProtocol }
                cancel(); c.mu.Lock(); c.closed=true; c.mu.Unlock()
                // Cancel has no acknowledgement. Host proceeds to shutdown and
                // forced cleanup; a second terminal result would be ambiguous.
                finished=true
            case "shutdown":
                cancel(); c.mu.Lock(); c.closed=true; c.mu.Unlock()
                return out.send(map[string]any{"type":"shutdown_ack"})
            case "host_result","host_error":
                c.mu.Lock()
                valid:=!finished && m.Invocation==c.identity.Invocation && len(c.readGate)==1 && m.Request==c.requests &&
                    (m.Type!="host_error" || m.Code=="denied")
                c.mu.Unlock()
                if !valid { return ErrProtocol }
                select { case c.replies<-m: default:return ErrProtocol }
            default:return ErrProtocol
            }
        }
    }
}

func readFrame(r *bufio.Reader)([]byte,error) {
    var frame []byte
    for {
        part,err:=r.ReadSlice('\n')
        if len(frame)+len(part)>MaxFrame+1 { return nil,ErrProtocol }
        frame=append(frame,part...)
        if err==bufio.ErrBufferFull { continue }
        if err!=nil { return nil,err }
        frame=frame[:len(frame)-1]
        if bytes.ContainsRune(frame,'\r') { return nil,ErrProtocol }
        if _,err=strict(frame);err!=nil {return nil,err}
        return frame,nil
    }
}
// Token walk rejects duplicate keys and limits complexity before a tree is built.
func strict(data []byte)(any,error) {
    if !utf8.Valid(data) || len(data)>MaxFrame { return nil,ErrProtocol }
    d:=json.NewDecoder(bytes.NewReader(data));d.UseNumber();nodes:=4096
    var walk func(int)(any,error)
    walk=func(depth int)(any,error){
        nodes--; if depth>48 || nodes<0 { return nil,ErrProtocol }
        token,err:=d.Token();if err!=nil { return nil,ErrProtocol }
        if delim,ok:=token.(json.Delim);ok {
            switch delim {
            case '{':
                object:=map[string]any{}
                for d.More(){
                    key,err:=d.Token();if err!=nil{return nil,ErrProtocol};name,ok:=key.(string);if !ok{return nil,ErrProtocol}
                    if _,ok:=object[name];ok{return nil,ErrProtocol}
                    v,err:=walk(depth+1);if err!=nil{return nil,err};object[name]=v
                }
                end,err:=d.Token();if err!=nil || end!=json.Delim('}') {return nil,ErrProtocol};return object,nil
            case '[':
                array:=[]any{}
                for d.More(){v,err:=walk(depth+1);if err!=nil{return nil,err};array=append(array,v)}
                end,err:=d.Token();if err!=nil || end!=json.Delim(']'){return nil,ErrProtocol};return array,nil
            default:return nil,ErrProtocol
            }
        }
        return token,nil
    }
    value,err:=walk(0);if err!=nil{return nil,err}
    if _,err=d.Token();err!=io.EOF{return nil,ErrProtocol};return value,nil
}

// Direction-specific exact field sets complement the wire schema. Optional
// fields from another variant are not silently accepted by the union decoder.
func validFields(frame []byte) bool {
    var fields map[string]json.RawMessage
    if json.Unmarshal(frame,&fields)!=nil{return false}
    var kind string
    if json.Unmarshal(fields["type"],&kind)!=nil{return false}
    sets:=map[string][]string{
        "initialize":{"type","protocol","identity","definitions","capabilities"},
        "invoke":{"type","invocation","kind","name","arguments"},
        "cancel":{"type","invocation"},
        "shutdown":{"type"},
        "host_result":{"type","invocation","request","text"},
        "host_error":{"type","invocation","request","code"},
    }
    keys,ok:=sets[kind];if !ok || len(keys)!=len(fields){return false}
    for _,key:=range keys {if _,ok:=fields[key];!ok{return false}}
    return true
}
