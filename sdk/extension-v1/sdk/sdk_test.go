package sdk

import (
    "bufio"
    "strings"
    "testing"
)

func TestStrictJSON(t *testing.T) {
    for _,input:=range []string{`{"a":1,"a":2}`,`{} {}`,strings.Repeat("[",49)+"0"+strings.Repeat("]",49),"["+strings.Repeat("0,",4096)+"0]"} {
        if _,err:=strict([]byte(input));err==nil {t.Fatal("accepted invalid JSON")}
    }
    if _,err:=strict([]byte(`{"a":[1,null,true,"text"]}`));err!=nil {t.Fatal(err)}
}
func TestFraming(t *testing.T) {
    for _,input:=range []string{"{}","{}\r\n",strings.Repeat("x",MaxFrame+1)+"\n"} {
        if _,err:=readFrame(bufio.NewReader(strings.NewReader(input)));err==nil {t.Fatal("accepted invalid frame")}
    }
    reader:=bufio.NewReader(strings.NewReader("{}\n[]\n"))
    if _,err:=readFrame(reader);err!=nil {t.Fatal(err)}
    if _,err:=readFrame(reader);err!=nil {t.Fatal(err)}
}
func TestVariantFields(t *testing.T) {
    if !validFields([]byte(`{"type":"shutdown"}`)) {t.Fatal("refused shutdown")}
    if validFields([]byte(`{"type":"shutdown","arguments":{}}`)) {t.Fatal("accepted cross-variant field")}
    if validFields([]byte(`{"type":"cancel"}`)) {t.Fatal("accepted missing identity")}
}
