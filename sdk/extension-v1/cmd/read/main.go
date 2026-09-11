package main

import (
    "context"
    _ "embed"
    "encoding/json"
    "errors"
    "os"
    "voyage.example/extension-v1/sdk"
)

//go:embed definitions.json
var definitions []byte
func main(){
    err:=sdk.Serve(definitions,[]string{"execute","host.file.read"},func(ctx context.Context,c *sdk.Call,kind,name string,args json.RawMessage)(any,error){
        if kind!="tool" || name!="read_text" {return nil,errors.New("unsupported")}
        var input struct{Path string `json:"path"`}
        if err:=json.Unmarshal(args,&input);err!=nil{return nil,err}
        text,err:=c.Read(ctx,input.Path,0,4096)
        if err!=nil{return nil,err}
        return map[string]string{"text":text},nil
    })
    if err!=nil{os.Exit(1)}
}
