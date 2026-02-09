package agentfunctions

import (
	"archive/zip"
	"bytes"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	agentstructs "github.com/MythicMeta/MythicContainer/agent_structs"
	"github.com/MythicMeta/MythicContainer/mythicrpc"
	"github.com/google/uuid"
	"golang.org/x/exp/slices"
)

const version = "0.1.0"

type sleepInfoStruct struct {
	Interval int       `json:"interval"`
	Jitter   int       `json:"jitter"`
	KillDate time.Time `json:"killdate"`
}

var payloadDefinition = agentstructs.PayloadType{
	Name:                                   "sebastian",
	SemVer:                                 version,
	FileExtension:                          "bin",
	Author:                                 "@xorrior, @djhohnstein, @Ne0nd0g, @its_a_feature_",
	SupportedOS:                            []string{agentstructs.SUPPORTED_OS_LINUX, agentstructs.SUPPORTED_OS_MACOS},
	Wrapper:                                false,
	CanBeWrappedByTheFollowingPayloadTypes: []string{},
	SupportsDynamicLoading:                 false,
	Description:                            fmt.Sprintf("A fully featured macOS and Linux Rust agent.\nNeeds Mythic 3.3.0+\nVersion: %s", version),
	SupportedC2Profiles:                    []string{"http", "websocket", "tcp", "dynamichttp", "webshell", "httpx", "dns"},
	MythicEncryptsData:                     true,
	BuildParameters: []agentstructs.BuildParameter{
		{
			Name:          "mode",
			Description:   "Choose the build mode option. Select default for executables, c-shared for a .dylib or .so file, or c-archive for a .zip containing a static library and header file",
			Required:      false,
			DefaultValue:  "default",
			Choices:       []string{"default", "c-archive", "c-shared"},
			ParameterType: agentstructs.BUILD_PARAMETER_TYPE_CHOOSE_ONE,
			UiPosition:    1,
		},
		{
			Name:          "architecture",
			Description:   "Choose the agent's architecture",
			Required:      false,
			DefaultValue:  "AMD_x64",
			Choices:       []string{"AMD_x64", "ARM_x64"},
			ParameterType: agentstructs.BUILD_PARAMETER_TYPE_CHOOSE_ONE,
			UiPosition:    2,
		},
		{
			Name:          "debug",
			Description:   "Create a debug build with print statements for debugging.",
			Required:      false,
			DefaultValue:  false,
			ParameterType: agentstructs.BUILD_PARAMETER_TYPE_BOOLEAN,
			UiPosition:    3,
		},
		{
			Name:          "strip",
			Description:   "Strip debug symbols from the output binary to reduce size.",
			Required:      false,
			DefaultValue:  true,
			ParameterType: agentstructs.BUILD_PARAMETER_TYPE_BOOLEAN,
			UiPosition:    4,
		},
		{
			Name:          "static",
			Description:   "Statically compile the payload (Linux only)",
			Required:      false,
			ParameterType: agentstructs.BUILD_PARAMETER_TYPE_BOOLEAN,
			DefaultValue:  false,
			SupportedOS:   []string{agentstructs.SUPPORTED_OS_LINUX},
			UiPosition:    5,
		},
		{
			Name:          "egress_order",
			Description:   "Prioritize the order in which egress connections are made (if including multiple egress c2 profiles)",
			Required:      false,
			ParameterType: agentstructs.BUILD_PARAMETER_TYPE_ARRAY,
			DefaultValue:  []string{"http", "websocket", "dynamichttp", "httpx"},
			GroupName:     "egress",
			UiPosition:    6,
		},
		{
			Name:          "egress_failover",
			Description:   "How should egress mechanisms rotate",
			Required:      false,
			ParameterType: agentstructs.BUILD_PARAMETER_TYPE_CHOOSE_ONE,
			Choices:       []string{"failover"},
			DefaultValue:  "failover",
			GroupName:     "egress",
			UiPosition:    7,
		},
		{
			Name:          "failover_threshold",
			Description:   "How many failed attempts should cause a rotate of egress comms",
			Required:      false,
			ParameterType: agentstructs.BUILD_PARAMETER_TYPE_NUMBER,
			DefaultValue:  10,
			GroupName:     "egress",
			UiPosition:    8,
		},
		{
			Name:          "proxy_bypass",
			Description:   "Ignore HTTP proxy environment settings configured on the target host?",
			Required:      false,
			DefaultValue:  false,
			ParameterType: agentstructs.BUILD_PARAMETER_TYPE_BOOLEAN,
			GroupName:     "egress",
			UiPosition:    9,
		},
	},
	SupportsMultipleC2InBuild: true,
	C2ParameterDeviations: map[string]map[string]agentstructs.C2ParameterDeviation{
		"http": {
			"get_uri": {
				Supported: false,
			},
			"query_path_name": {
				Supported: false,
			},
		},
	},
	BuildSteps: []agentstructs.BuildStep{
		{
			Name:        "Configuring",
			Description: "Cleaning up configuration values and generating the cargo build command",
		},
		{
			Name:        "Compiling",
			Description: "Compiling the Rust agent with cargo",
		},
	},
	CheckIfCallbacksAliveFunction: func(message agentstructs.PTCheckIfCallbacksAliveMessage) agentstructs.PTCheckIfCallbacksAliveMessageResponse {
		response := agentstructs.PTCheckIfCallbacksAliveMessageResponse{Success: true, Callbacks: make([]agentstructs.PTCallbacksToCheckResponse, 0)}
		for _, callback := range message.Callbacks {
			if callback.SleepInfo == "" {
				continue
			}
			sleepInfo := map[string]sleepInfoStruct{}
			err := json.Unmarshal([]byte(callback.SleepInfo), &sleepInfo)
			if err != nil {
				continue
			}
			atLeastOneCallbackWithinRange := false
			for activeC2 := range sleepInfo {
				if activeC2 == "websocket" && callback.LastCheckin.Unix() == 0 {
					atLeastOneCallbackWithinRange = true
					continue
				}
				if activeC2 == "tcp" {
					atLeastOneCallbackWithinRange = true
					continue
				}
				maxAdd := sleepInfo[activeC2].Interval
				if sleepInfo[activeC2].Jitter > 0 {
					maxAdd = maxAdd + ((sleepInfo[activeC2].Jitter / 100) * (sleepInfo[activeC2].Interval))
				}
				maxAdd *= 2
				latest := callback.LastCheckin.Add(time.Duration(maxAdd) * time.Second)
				if time.Now().UTC().Before(latest) {
					atLeastOneCallbackWithinRange = true
					break
				}
			}
			response.Callbacks = append(response.Callbacks, agentstructs.PTCallbacksToCheckResponse{
				ID:    callback.ID,
				Alive: atLeastOneCallbackWithinRange,
			})
		}
		return response
	},
}

func build(payloadBuildMsg agentstructs.PayloadBuildMessage) agentstructs.PayloadBuildResponse {
	payloadBuildResponse := agentstructs.PayloadBuildResponse{
		PayloadUUID:        payloadBuildMsg.PayloadUUID,
		Success:            true,
		UpdatedCommandList: &payloadBuildMsg.CommandList,
	}
	if len(payloadBuildMsg.C2Profiles) == 0 {
		payloadBuildResponse.Success = false
		payloadBuildResponse.BuildStdErr = "Failed to build - must select at least one C2 Profile"
		return payloadBuildResponse
	}

	targetOs := "linux"
	if payloadBuildMsg.SelectedOS == "macOS" {
		targetOs = "darwin"
	}

	egress_order, err := payloadBuildMsg.BuildParameters.GetArrayArg("egress_order")
	if err != nil {
		payloadBuildResponse.Success = false
		payloadBuildResponse.BuildStdErr = err.Error()
		return payloadBuildResponse
	}
	egress_failover, err := payloadBuildMsg.BuildParameters.GetChooseOneArg("egress_failover")
	if err != nil {
		payloadBuildResponse.Success = false
		payloadBuildResponse.BuildStdErr = err.Error()
		return payloadBuildResponse
	}
	debug, err := payloadBuildMsg.BuildParameters.GetBooleanArg("debug")
	if err != nil {
		payloadBuildResponse.Success = false
		payloadBuildResponse.BuildStdErr = err.Error()
		return payloadBuildResponse
	}
	static, err := payloadBuildMsg.BuildParameters.GetBooleanArg("static")
	if err != nil {
		payloadBuildResponse.Success = false
		payloadBuildResponse.BuildStdErr = err.Error()
		return payloadBuildResponse
	}
	if static && targetOs == "darwin" {
		payloadBuildResponse.Success = false
		payloadBuildResponse.BuildStdErr = "Cannot build fully static library for macOS"
		return payloadBuildResponse
	}
	failedConnectionCountThreshold, err := payloadBuildMsg.BuildParameters.GetNumberArg("failover_threshold")
	if err != nil {
		payloadBuildResponse.Success = false
		payloadBuildResponse.BuildStdErr = err.Error()
		return payloadBuildResponse
	}
	proxyBypass, err := payloadBuildMsg.BuildParameters.GetBooleanArg("proxy_bypass")
	if err != nil {
		payloadBuildResponse.Success = false
		payloadBuildResponse.BuildStdErr = err.Error()
		return payloadBuildResponse
	}
	architecture, err := payloadBuildMsg.BuildParameters.GetStringArg("architecture")
	if err != nil {
		payloadBuildResponse.Success = false
		payloadBuildResponse.BuildStdErr = err.Error()
		return payloadBuildResponse
	}
	mode, err := payloadBuildMsg.BuildParameters.GetStringArg("mode")
	if err != nil {
		payloadBuildResponse.Success = false
		payloadBuildResponse.BuildStdErr = err.Error()
		return payloadBuildResponse
	}
	strip, err := payloadBuildMsg.BuildParameters.GetBooleanArg("strip")
	if err != nil {
		payloadBuildResponse.Success = false
		payloadBuildResponse.BuildStdErr = err.Error()
		return payloadBuildResponse
	}

	// Build environment variables for the Rust agent's build.rs
	envVars := map[string]string{
		"AGENT_UUID":                         payloadBuildMsg.PayloadUUID,
		"DEBUG":                              fmt.Sprintf("%v", debug),
		"EGRESS_FAILOVER":                    egress_failover,
		"FAILED_CONNECTION_COUNT_THRESHOLD":  fmt.Sprintf("%v", failedConnectionCountThreshold),
		"PROXY_BYPASS":                       fmt.Sprintf("%v", proxyBypass),
	}

	if egressBytes, err := json.Marshal(egress_order); err != nil {
		payloadBuildResponse.Success = false
		payloadBuildResponse.BuildStdErr = err.Error()
		return payloadBuildResponse
	} else {
		envVars["EGRESS_ORDER"] = base64.StdEncoding.EncodeToString(egressBytes)
	}

	// Process C2 profile parameters
	for index := range payloadBuildMsg.C2Profiles {
		initialConfig := make(map[string]interface{})
		for _, key := range payloadBuildMsg.C2Profiles[index].GetArgNames() {
			if key == "AESPSK" {
				cryptoVal, err := payloadBuildMsg.C2Profiles[index].GetCryptoArg(key)
				if err != nil {
					payloadBuildResponse.Success = false
					payloadBuildResponse.BuildStdErr = "Key error: " + key + "\n" + err.Error()
					return payloadBuildResponse
				}
				initialConfig[key] = cryptoVal.EncKey
			} else if key == "headers" {
				headers, err := payloadBuildMsg.C2Profiles[index].GetDictionaryArg(key)
				if err != nil {
					payloadBuildResponse.Success = false
					payloadBuildResponse.BuildStdErr = "Key error: " + key + "\n" + err.Error()
					return payloadBuildResponse
				}
				initialConfig[key] = headers
			} else if key == "raw_c2_config" {
				agentConfigString, err := payloadBuildMsg.C2Profiles[index].GetStringArg(key)
				if err != nil {
					payloadBuildResponse.Success = false
					payloadBuildResponse.BuildStdErr = "Key error: " + key + "\n" + err.Error()
					return payloadBuildResponse
				}
				configData, err := mythicrpc.SendMythicRPCFileGetContent(mythicrpc.MythicRPCFileGetContentMessage{
					AgentFileID: agentConfigString,
				})
				if err != nil {
					payloadBuildResponse.Success = false
					payloadBuildResponse.BuildStdErr = "Key error: " + key + "\n" + err.Error()
					return payloadBuildResponse
				}
				if !configData.Success {
					payloadBuildResponse.Success = false
					payloadBuildResponse.BuildStdErr = "Key error: " + key + "\n" + configData.Error
					return payloadBuildResponse
				}
				tomlConfig := make(map[string]interface{})
				err = json.Unmarshal(configData.Content, &tomlConfig)
				if err != nil {
					payloadBuildResponse.Success = false
					payloadBuildResponse.BuildStdErr = "Key error: " + key + "\n" + err.Error()
					return payloadBuildResponse
				}
				initialConfig[key] = tomlConfig
			} else if slices.Contains([]string{"callback_jitter", "callback_interval", "callback_port", "port", "failover_threshold", "max_query_length", "max_subdomain_length"}, key) {
				val, err := payloadBuildMsg.C2Profiles[index].GetNumberArg(key)
				if err != nil {
					stringVal, err := payloadBuildMsg.C2Profiles[index].GetStringArg(key)
					if err != nil {
						payloadBuildResponse.Success = false
						payloadBuildResponse.BuildStdErr = "Key error: " + key + "\n" + err.Error()
						return payloadBuildResponse
					}
					realVal, err := strconv.Atoi(stringVal)
					if err != nil {
						payloadBuildResponse.Success = false
						payloadBuildResponse.BuildStdErr = "Key error: " + key + "\n" + err.Error()
						return payloadBuildResponse
					}
					initialConfig[key] = realVal
				} else {
					initialConfig[key] = int(val)
				}
			} else if slices.Contains([]string{"encrypted_exchange_check"}, key) {
				val, err := payloadBuildMsg.C2Profiles[index].GetBooleanArg(key)
				if err != nil {
					stringVal, err := payloadBuildMsg.C2Profiles[index].GetStringArg(key)
					if err != nil {
						payloadBuildResponse.Success = false
						payloadBuildResponse.BuildStdErr = "Key error: " + key + "\n" + err.Error()
						return payloadBuildResponse
					}
					initialConfig[key] = stringVal == "T"
				} else {
					initialConfig[key] = val
				}
			} else if slices.Contains([]string{"callback_domains", "domains"}, key) {
				val, err := payloadBuildMsg.C2Profiles[index].GetArrayArg(key)
				if err != nil {
					payloadBuildResponse.Success = false
					payloadBuildResponse.BuildStdErr = "Key error: " + key + "\n" + err.Error()
					return payloadBuildResponse
				}
				initialConfig[key] = val
			} else {
				val, err := payloadBuildMsg.C2Profiles[index].GetStringArg(key)
				if err != nil {
					payloadBuildResponse.Success = false
					payloadBuildResponse.BuildStdErr = "Key error: " + key + "\n" + err.Error()
					return payloadBuildResponse
				}
				if key == "proxy_port" {
					if val == "" {
						initialConfig[key] = 0
					} else {
						intval, err := strconv.Atoi(val)
						if err != nil {
							payloadBuildResponse.Success = false
							payloadBuildResponse.BuildStdErr = "Key error: " + key + "\n" + err.Error()
							return payloadBuildResponse
						}
						initialConfig[key] = intval
					}
				} else {
					initialConfig[key] = val
				}
			}
		}
		initialConfigBytes, err := json.Marshal(initialConfig)
		if err != nil {
			payloadBuildResponse.Success = false
			payloadBuildResponse.BuildStdErr = err.Error()
			return payloadBuildResponse
		}
		initialConfigBase64 := base64.StdEncoding.EncodeToString(initialConfigBytes)
		payloadBuildResponse.BuildStdOut += fmt.Sprintf("%s's config: \n%v\n", payloadBuildMsg.C2Profiles[index].Name, string(initialConfigBytes))
		envKey := fmt.Sprintf("C2_%s_INITIAL_CONFIG", strings.ToUpper(payloadBuildMsg.C2Profiles[index].Name))
		envVars[envKey] = initialConfigBase64
	}

	// Determine Rust target triple
	rustArch := "x86_64"
	if architecture == "ARM_x64" {
		rustArch = "aarch64"
	}

	var rustTarget string
	if targetOs == "darwin" {
		rustTarget = fmt.Sprintf("%s-apple-darwin", rustArch)
	} else {
		if static {
			rustTarget = fmt.Sprintf("%s-unknown-linux-musl", rustArch)
		} else {
			rustTarget = fmt.Sprintf("%s-unknown-linux-gnu", rustArch)
		}
	}

	// Determine crate type based on mode
	crateType := ""
	switch mode {
	case "c-shared":
		crateType = "cdylib"
	case "c-archive":
		crateType = "staticlib"
	default:
		crateType = "bin"
	}

	// Build the cargo command
	// Use cargo-zigbuild for macOS targets (provides cross-compilation C compiler)
	cargoCmd := "cargo"
	cargoArgs := []string{"build", "--release", "--target", rustTarget}
	if targetOs == "darwin" {
		cargoCmd = "cargo"
		cargoArgs = []string{"zigbuild", "--release", "--target", rustTarget}
	}
	if crateType != "bin" {
		// For library builds, we need to set the crate type
		// The Cargo.toml should have both bin and lib targets
		cargoArgs = append(cargoArgs, "--lib")
	}

	// Build RUSTFLAGS
	rustflags := ""
	if strip {
		rustflags += "-C strip=symbols "
	}
	if static && targetOs == "linux" {
		rustflags += "-C target-feature=+crt-static "
	}
	// Set cross-compilation linker for Linux targets
	if targetOs == "linux" {
		if rustArch == "aarch64" {
			rustflags += "-C linker=aarch64-linux-gnu-gcc "
		} else {
			rustflags += "-C linker=x86_64-linux-gnu-gcc "
		}
	}

	// Build the output path
	payloadName := fmt.Sprintf("%s-%s-%s", payloadBuildMsg.PayloadUUID, targetOs, rustArch)
	extension := ""
	if mode == "c-shared" {
		if targetOs == "darwin" {
			extension = ".dylib"
		} else {
			extension = ".so"
		}
	} else if mode == "c-archive" {
		extension = ".a"
	}
	payloadName += extension

	mythicrpc.SendMythicRPCPayloadUpdateBuildStep(mythicrpc.MythicRPCPayloadUpdateBuildStepMessage{
		PayloadUUID: payloadBuildMsg.PayloadUUID,
		StepName:    "Configuring",
		StepSuccess: true,
		StepStdout:  fmt.Sprintf("Successfully configured\nTarget: %s\nMode: %s\nCrate type: %s\n", rustTarget, mode, crateType),
	})

	// Execute cargo build
	cmd := exec.Command(cargoCmd, cargoArgs...)
	cmd.Dir = "./sebastian/agent_code/"

	// Set environment variables
	cmd.Env = os.Environ()
	for k, v := range envVars {
		cmd.Env = append(cmd.Env, fmt.Sprintf("%s=%s", k, v))
	}
	if rustflags != "" {
		cmd.Env = append(cmd.Env, fmt.Sprintf("RUSTFLAGS=%s", strings.TrimSpace(rustflags)))
	}
	if crateType != "bin" {
		cmd.Env = append(cmd.Env, fmt.Sprintf("SEBASTIAN_CRATE_TYPE=%s", crateType))
	}

	var stdout bytes.Buffer
	var stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr

	if err := cmd.Run(); err != nil {
		payloadBuildResponse.Success = false
		payloadBuildResponse.BuildMessage = "Compilation failed with errors"
		payloadBuildResponse.BuildStdErr += stderr.String() + "\n" + err.Error()
		payloadBuildResponse.BuildStdOut += stdout.String()
		mythicrpc.SendMythicRPCPayloadUpdateBuildStep(mythicrpc.MythicRPCPayloadUpdateBuildStepMessage{
			PayloadUUID: payloadBuildMsg.PayloadUUID,
			StepName:    "Compiling",
			StepSuccess: false,
			StepStdout:  fmt.Sprintf("failed to compile\n%s\n%s\n%s", stderr.String(), stdout.String(), err.Error()),
		})
		return payloadBuildResponse
	}

	mythicrpc.SendMythicRPCPayloadUpdateBuildStep(mythicrpc.MythicRPCPayloadUpdateBuildStepMessage{
		PayloadUUID: payloadBuildMsg.PayloadUUID,
		StepName:    "Compiling",
		StepSuccess: true,
		StepStdout:  fmt.Sprintf("Successfully compiled\n%s\n%s", stdout.String(), stderr.String()),
	})
	payloadBuildResponse.BuildStdErr = stderr.String()
	payloadBuildResponse.BuildStdOut += stdout.String()

	// Determine the output artifact path
	artifactDir := fmt.Sprintf("./sebastian/agent_code/target/%s/release/", rustTarget)
	var artifactPath string
	if crateType == "bin" {
		artifactPath = filepath.Join(artifactDir, "sebastian")
	} else if crateType == "cdylib" {
		if targetOs == "darwin" {
			artifactPath = filepath.Join(artifactDir, "libsebastian.dylib")
		} else {
			artifactPath = filepath.Join(artifactDir, "libsebastian.so")
		}
	} else if crateType == "staticlib" {
		artifactPath = filepath.Join(artifactDir, "libsebastian.a")
	}

	payloadBytes, err := os.ReadFile(artifactPath)
	if err != nil {
		payloadBuildResponse.Success = false
		payloadBuildResponse.BuildMessage = "Failed to find final payload"
		payloadBuildResponse.BuildStdErr += fmt.Sprintf("\n%v\n", err)
		return payloadBuildResponse
	}

	if mode == "c-archive" {
		// Package as zip with .a, .h, and sharedlib .c
		zipUUID := uuid.New().String()
		archive, err := os.Create(fmt.Sprintf("/build/%s", zipUUID))
		if err != nil {
			payloadBuildResponse.Success = false
			payloadBuildResponse.BuildMessage = "Failed to make temp archive on disk"
			payloadBuildResponse.BuildStdErr += fmt.Sprintf("\n%v\n", err)
			return payloadBuildResponse
		}
		zipWriter := zip.NewWriter(archive)

		archiveName := fmt.Sprintf("sebastian-%s-%s.a", targetOs, rustArch)
		fileWriter, err := zipWriter.Create(archiveName)
		if err != nil {
			payloadBuildResponse.Success = false
			payloadBuildResponse.BuildMessage = "Failed to save payload to zip"
			archive.Close()
			return payloadBuildResponse
		}
		_, err = io.Copy(fileWriter, bytes.NewReader(payloadBytes))
		if err != nil {
			payloadBuildResponse.Success = false
			payloadBuildResponse.BuildMessage = "Failed to write payload to zip"
			archive.Close()
			return payloadBuildResponse
		}

		// Add a header file for FFI usage
		headerContent := `#ifndef SEBASTIAN_H
#define SEBASTIAN_H

extern void run_main(void);

#endif /* SEBASTIAN_H */
`
		headerWriter, err := zipWriter.Create(fmt.Sprintf("sebastian-%s-%s.h", targetOs, rustArch))
		if err != nil {
			payloadBuildResponse.Success = false
			payloadBuildResponse.BuildMessage = "Failed to save header to zip"
			archive.Close()
			return payloadBuildResponse
		}
		_, err = headerWriter.Write([]byte(headerContent))
		if err != nil {
			payloadBuildResponse.Success = false
			payloadBuildResponse.BuildMessage = "Failed to write header to zip"
			archive.Close()
			return payloadBuildResponse
		}

		// Add sharedlib loader
		sharedLibContent := `#include <stdio.h>
#include "sebastian.h"

int main() {
    run_main();
    return 0;
}
`
		sharedWriter, err := zipWriter.Create("sharedlib-loader.c")
		if err != nil {
			payloadBuildResponse.Success = false
			payloadBuildResponse.BuildMessage = "Failed to save sharedlib to zip"
			archive.Close()
			return payloadBuildResponse
		}
		_, err = sharedWriter.Write([]byte(sharedLibContent))
		if err != nil {
			payloadBuildResponse.Success = false
			payloadBuildResponse.BuildMessage = "Failed to write sharedlib to zip"
			archive.Close()
			return payloadBuildResponse
		}

		zipWriter.Close()
		archive.Close()

		archiveBytes, err := os.ReadFile(fmt.Sprintf("/build/%s", zipUUID))
		if err != nil {
			payloadBuildResponse.Success = false
			payloadBuildResponse.BuildMessage = "Failed to read final zip"
			return payloadBuildResponse
		}
		payloadBuildResponse.Payload = &archiveBytes
		payloadBuildResponse.Success = true
		payloadBuildResponse.BuildMessage = "Successfully built payload!"
		if !strings.HasSuffix(payloadBuildMsg.Filename, ".zip") {
			updatedFilename := fmt.Sprintf("%s.zip", payloadBuildMsg.Filename)
			payloadBuildResponse.UpdatedFilename = &updatedFilename
		}
	} else {
		payloadBuildResponse.Payload = &payloadBytes
		payloadBuildResponse.Success = true
		payloadBuildResponse.BuildMessage = "Successfully built payload!"
	}

	return payloadBuildResponse
}

func onNewCallback(data agentstructs.PTOnNewCallbackAllData) agentstructs.PTOnNewCallbackResponse {
	return agentstructs.PTOnNewCallbackResponse{
		AgentCallbackID: data.Callback.AgentCallbackID,
		Success:         true,
		Error:           "",
	}
}

func Initialize() {
	agentstructs.AllPayloadData.Get("sebastian").AddPayloadDefinition(payloadDefinition)
	agentstructs.AllPayloadData.Get("sebastian").AddBuildFunction(build)
	agentstructs.AllPayloadData.Get("sebastian").AddIcon(filepath.Join(".", "sebastian", "agentfunctions", "sebastian.svg"))
}
