package main

import (
	"bufio"
	"crypto/aes"
	"crypto/cipher"
	"crypto/md5"
	"crypto/rand"
	"encoding/hex"
	"io"

	// "io"
	// "log"
	"fmt"
	"os"
	"os/exec"
	"regexp"
	"strconv"
	"strings"

	"github.com/gdamore/tcell/v2"
	"github.com/rivo/tview"
)

var args []string
var buildTime string
var commsFromFile string
var scripts string
var regexBashShebang string = "#!/(usr/bin|bin)/(bash|sh)"
var textView = tview.NewTextView()
var app = tview.NewApplication()
var list = tview.NewList()
var comms []string
var commsMod []string
var isSudoSkippable bool = false
var commandPrintOnly bool = false
var isScript bool

func createHash(key string) string {
	hasher := md5.New()
	hasher.Write([]byte(key))
	//fmt.Println(hex.EncodeToString(hasher.Sum(nil)))
	return hex.EncodeToString(hasher.Sum(nil))
}

func encrypt(data []byte, passphrase string) []byte {
	block, _ := aes.NewCipher([]byte(createHash(passphrase)))
	gcm, err := cipher.NewGCM(block)
	if err != nil {
		panic(err.Error())
	}
	nonce := make([]byte, gcm.NonceSize())
	if _, err = io.ReadFull(rand.Reader, nonce); err != nil {
		panic(err.Error())
	}
	ciphertext := gcm.Seal(nonce, nonce, data, nil)
	return ciphertext
}

func decrypt(data []byte, passphrase string) []byte {
	key := []byte(createHash(passphrase))
	block, err := aes.NewCipher(key)
	if err != nil {
		panic(err.Error())
	}
	gcm, err := cipher.NewGCM(block)
	if err != nil {
		panic(err.Error())
	}
	nonceSize := gcm.NonceSize()
	nonce, ciphertext := data[:nonceSize], data[nonceSize:]
	plaintext, err := gcm.Open(nil, nonce, ciphertext, nil)
	if err != nil {
		panic(err.Error())
	}
	return plaintext
}
func linesToCommands(stri []string) []string {
	var r []string
	var lastComment = ""
	var appendd = ""
	for _, str := range stri {
		if str == "" {
			lastComment = ""
		} else if strings.Index(str, "#") == 0 {
			runes := []rune(str)
			lastComment = strings.TrimSpace(string(runes[1:]))
		} else {
			appendd = ""
			if len(lastComment) > 0 {
				appendd = lastComment + "^^"
			}
			r = append(r, appendd+str)
			lastComment = ""
		}
	}
	return r
}
func RemoveIndex(s []string, index int) []string {
	return append(s[:index], s[index+1:]...)
}
func Find(slice []string, val string) (int, bool) {
	for i, item := range slice {
		if item == val {
			return i, true
		}
	}
	return -1, false
}
func isRoot() bool {
	stdout, err := exec.Command("ps", "-o", "user=", "-p", strconv.Itoa(os.Getpid())).Output()
	if err != nil {
		//fmt.Println(err)
		//os.Exit(1)
	}
	// fmt.Println(string(stdout))
	return strings.Replace(string(stdout), "\n", "", -1) == "root"
}
func scanInput() string {

	input := bufio.NewScanner(os.Stdin) //Creating a Scanner that will read the input from the console

	for input.Scan() {
		//fmt.Println("Entered bytes:", input.Bytes())
		//if input.Text() == "\r" { break } //Break out of input loop when the user types the word "end"
		//fmt.Println("--" + input.Text())
		break
		//return input.Text()
	}
	return input.Text()
}
func executeCommand(command string) {

	if isRoot() {
		fmt.Println("\u001b[31mrunning as root\u001b[0m")
	}

	// check for bash if available
	useShell := "bash"
	var labelTagsSL []string
	lookPath, err := exec.LookPath(useShell)
	if err != nil {
		//fmt.Println("bash not found, using shell")
		useShell = "sh"
		lookPath, _ = exec.LookPath(useShell)
	}
	labelsS := strings.Split(command, "^^")

	if len(labelsS) > 1 {
		labelTagsSL = strings.Split(labelsS[0], " ")
		labelsS = RemoveIndex(labelsS, 0)
		command = strings.Join(labelsS, "^^")
	}
	_, hasSudo := Find(labelTagsSL, "sudo")
	if isSudoSkippable {
	} else if hasSudo && !isRoot() {
		fmt.Println("Command: " + command)
		fmt.Println("Tags: " + strings.Join(labelTagsSL, " "))
		fmt.Println("\u001B[31mCommand must be executed as root !\u001B[0m")
		return
	}

	// find prompts
	re := regexp.MustCompile(`(<prompt:.*?>)`)
	reSplit := re.FindAllString(command, -1)
	var promptArr []string

	for i := range reSplit {
		_, found := Find(promptArr, reSplit[i])
		if !found {
			promptArr = append(promptArr, reSplit[i])
		}
	}
	//fmt.Print(promptArr)

	if len(promptArr) > 0 {
		fmt.Println("\033[33m" + command + "\033[0m")
		var strSplit []string
		var read string
		var out string
		var stri string
		for i := 0; i < len(promptArr); i++ {
			stri = strings.Trim(promptArr[i], "<>")
			strSplit = strings.Split(stri, ":")
			//useBooleanPrompt := false
			out = ""
			read = ""
			if len(strSplit) == 4 {
				//useBooleanPrompt = true
				fmt.Print(strSplit[1])
				fmt.Print(" [Y]es\u001B[32m" +
					//strSplit[2]+
					"\u001B[0m / [n]o \u001B[31m" +
					//strSplit[3]+
					"\u001B[0m: ")
				_, _ = fmt.Scanln(&read)
				if read == "" || strings.ToLower(read) == "y" || strings.ToLower(read) == "yes" {
					out = strSplit[2]
				} else {
					out = strSplit[3]
				}
				//fmt.Print(": ")
			} else {
				fmt.Print(strSplit[1]) // print info label
				if len(strSplit) > 2 {
					fmt.Print(" (" + strSplit[2] + ")") // default value
				}
				fmt.Print(": ")
				//_, _ = fmt.Scanln(&read) // wait for user input
				read = scanInput()
				if read == "" && len(strSplit) > 2 { // empty input is default value
					//fmt.Println("def val")
					out = strSplit[2]
				} else {
					out = read // set value from input
					//fmt.Println("set value from input")
				}
				//fmt.Println("Scanln", read, out)
			}
			//fmt.Println(promptArr[i], out)
			command = strings.Replace(command, promptArr[i], out, -1)
			//fmt.Print(first)
		}
		//fmt.Print(promptArr)
	}

	commandPrint := command
	//skipExecution := false
	/*if (len(args) > 0 && args[0] == "print") {
		fmt.Println("\033[33m" + command + "\033[0m")
		//skipExecution = true
	} else*/if len(command) > 200 && !commandPrintOnly {
		commandPrint = command[0:200] + " ... + " + fmt.Sprintf("%d script chars", len(command)-200)
	}

	// add shebang back for scripts
	commandToPrint := "#!" + lookPath + "\n" + strings.TrimSpace(commandPrint)
	//rege := regexp.MustCompile(regexBashShebang)
	//splitLines := strings.Split(strings.TrimSpace(commandPrint), "\n")
	//fmt.Println("commandPrintOnly:", commandPrintOnly)
	//fmt.Println("splitLines:", splitLines[0], splitLines[1])
	//ret := rege.Find([]byte(splitLines[0]))
	//commandToPrint = "#!/bin/bash" + commandToPrint

	fmt.Println("\033[33m" + commandToPrint + "\033[0m")

	if !commandPrintOnly {
		cmd := exec.Command(useShell, "-c", command)

		cmd.Stdout = os.Stdout
		cmd.Stdin = os.Stdin
		cmd.Stderr = os.Stderr
		_ = cmd.Run()
	}
	//}
	// fmt.Print
	/* stdin, err := cmd.StdinPipe()
	if err != nil {
		log.Fatal(err)
	}
	// go func() {
		defer stdin.Close()
		// io.WriteString(stdin, "values written to stdin are passed to cmd's standard input")
	// }()

	stdout, err := cmd.CombinedOutput()

	if err != nil {
		fmt.Print(string(stdout))
		fmt.Println(err.Error())
		return
	}

	fmt.Print(string(stdout)) */

}
func searchRender(search string) {
	list.Clear()
	var split []string
	searchSplit := strings.Split(search, " ")
	//keywords := []string{}
	commsMod = comms
	miss := false
	var commColor = "[green]"
	if isRoot() {
		commColor = "[red]"
	}
	// fmt.Println(len(searchSplit));
	// fmt.Println(len(search));
	if len(searchSplit) > 0 && len(search) > 0 {

		commsMod = []string{}
		for n := 0; n < len(comms); n++ {
			miss = false
			for i := 0; i < len(searchSplit); i++ {
				if !strings.Contains(comms[n], searchSplit[i]) && len(searchSplit[i]) > 0 {
					miss = true
					break
				}
			}
			if !miss {
				commsMod = append(commsMod, comms[n])
			}
		}
	}

	for i := 0; i < len(commsMod); i++ {
		toParse := commsMod[i]
		//toParse = ""
		split = strings.Split(toParse, "^^")
		if len(split) > 1 {
			toParse = "[white]" + split[0] + "[grey]^^" + commColor
			split = RemoveIndex(split, 0)
			toParse += strings.Join(split, "^^")
		} else {
			toParse = commColor + strings.Join(split, "^^")
		}

		list.AddItem(toParse, "", 0, nil)
		//fmt.Println(comms[i]);
	}
}
func main() {

	removedNewLines := strings.Replace(commsFromFile, "\r\n", "\n", -1)

	replacedTicks := strings.Replace(removedNewLines, "_____single-tick_____", "'", -1)
	comms = linesToCommands(strings.Split(replacedTicks, "\n"))

	scriptLines := strings.Replace(scripts, "_____single-tick_____", "'", -1)
	rege := regexp.MustCompile(regexBashShebang)
	split := rege.Split(scriptLines, -1)
	if strings.Trim(split[0], " ") == "" {
		split = split[1:]
	}
	for _, stri := range split {
		splitLines := strings.Split(stri, "\n")
		label := ""
		//fmt.Println("----")
		for _, line := range splitLines {
			//fmt.Println(len(line), line)
			if len(line) > 1 && line[:1] == "#" && label == "" {
				label = strings.Trim(line[1:], " ")
			}
		}
		stri = label + "^^" + strings.Join(splitLines, "\n")
		comms = append(comms, stri)
	}

	//scriptsSplits := strings.Split(scriptLines, "")
	//comms = append(comms, split...)

	//fmt.Println(comms)
	//os.Exit(99)
	args = os.Args[1:]
	if len(args) > 0 && args[0] == "print" {
		commandPrintOnly = true
		args = args[1:]
		//if len(args) == 0 {args = []string{""}}
	}
	list = tview.NewList().SetChangedFunc(func(index int, mainText string, secondaryText string, shortcut rune) {
		toParse := commsMod[index]
		if strings.Contains(toParse, "^^") {
			breakIndex := strings.Index(toParse, "^^") + 1
			toParse = string([]rune(toParse)[breakIndex:])
		}
		textView.SetText("\n[yellow]" + toParse)
	})
	inputField := tview.NewInputField()
	search := ""

	if len(args) > 0 {

		_, isSudoSkippable = Find(args, "--no-sudo-check")

		if args[0] == "-v" {
			fmt.Printf("Build Time: %s\n", buildTime)
			return
		} else if args[0] == "help" || args[0] == "-h" {
			fmt.Println("Parameters:\n   --no-sudo-check   skip sudo check for commands conatining sudo")
			fmt.Println("   -v   prints build time")
		} else if args[0] == "encryptaes" && len(args) > 2 {
			ciphertext := encrypt([]byte(args[2]), args[1])
			//fmt.Println(ciphertext)
			fmt.Printf("%x", ciphertext)
			return
		} else if args[0] == "decryptaes" && len(args) > 2 {
			decoded, err := hex.DecodeString(args[2])
			if err != nil {
				panic(err.Error())
			}
			//fmt.Println(decoded)
			plaintext := decrypt([]byte(decoded), args[1])
			fmt.Printf("%s", plaintext)
		}
		for i := 0; i < len(comms); i++ {
			if strings.Index(comms[i], args[0]+" ") == 0 || strings.Index(comms[i], args[0]+"^^") == 0 {
				executeCommand(comms[i])
				break
			}
		}
		return
	}

	//var key tcell.Key
	//var text string
	inputField.SetLabel("").SetPlaceholder("").SetFieldWidth(0).
		//SetAcceptanceFunc(tview.InputFieldInteger).
		SetChangedFunc(func(text string) {
			search = inputField.GetText()
			searchRender(search)
		}).SetDoneFunc(func(key tcell.Key) {
		if key == tcell.KeyEscape {
			app.Stop()
		} else if key == tcell.KeyEnter {
			app.Stop()
			// command, _ := list.GetItemText(list.GetCurrentItem())
			//fmt.Println(len(commsMod))
			command := commsMod[list.GetCurrentItem()]
			executeCommand(command)

		} else if key == tcell.KeyPgDn {
			list.SetCurrentItem(list.GetCurrentItem() + 20)
		} else if key == tcell.KeyDown {
			if list.GetCurrentItem() >= list.GetItemCount()-1 {
				list.SetCurrentItem(0)
			} else {
				list.SetCurrentItem(list.GetCurrentItem() + 1)
			}
		} else if key == tcell.KeyUp {
			list.SetCurrentItem(list.GetCurrentItem() - 1)
		} else {

		}
	})

	list.ShowSecondaryText(false)
	searchRender(search)
	textView.SetDynamicColors(true).
		SetBorder(false)
		//SetRegions(false).
		//SetWordWrap(true).
		/* SetChangedFunc(func() {
			app.Draw()
		}) */
	//box.SetBorder(true).SetBorderAttributes(tcell.AttrBold)

	flex := tview.NewFlex().SetDirection(tview.FlexRow).
		AddItem(inputField, 1, 1, true).
		//AddItem(tview.NewFlex().SetDirection(tview.FlexRow).
		AddItem(list, 0, 1, false).
		AddItem(textView, 5, 1, false)

	if err := app.SetRoot(flex, true).EnableMouse(false).Run(); err != nil {
		//panic(err)
	}

}
