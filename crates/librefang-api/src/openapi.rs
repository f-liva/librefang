{
  "openapi": "3.0.3",
  "info": {
    "title": "LibreFang Agent OS API",
    "description": "REST API for controlling LibreFang Agent OS",
    "version": "1.0.0"
  },
  "servers": [
    {
      "url": "http://localhost:4545",
      "description": "Local development server"
    }
  ],
  "paths": {
    "/api/health": {
      "get": {
        "summary": "Health check",
        "operationId": "health",
        "responses": {
          "200": {
            "description": "Server is healthy",
            "content": {
              "application/json": {
                "schema": {
                  "type": "object",
                  "properties": {
                    "status": {
                      "type": "string"
                    }
                  }
                }
              }
            }
          }
        }
      }
    },
    "/api/agents": {
      "get": {
        "summary": "List all agents",
        "operationId": "listAgents",
        "responses": {
          "200": {
            "description": "List of agents",
            "content": {
              "application/json": {
                "schema": {
                  "type": "array",
                  "items": {
                    "$ref": "#/components/schemas/Agent"
                  }
                }
              }
            }
          }
        }
      },
      "post": {
        "summary": "Create an agent",
        "operationId": "createAgent",
        "requestBody": {
          "required": true,
          "content": {
            "application/json": {
              "schema": {
                "type": "object"
              }
            }
          }
        },
        "responses": {
          "200": {
            "description": "Created agent",
            "content": {
              "application/json": {
                "schema": {
                  "$ref": "#/components/schemas/Agent"
                }
              }
            }
          }
        }
      }
    },
    "/api/agents/{id}": {
      "get": {
        "summary": "Get agent by ID",
        "operationId": "getAgent",
        "parameters": [
          {
            "name": "id",
            "in": "path",
            "required": true,
            "schema": {
              "type": "string"
            }
          }
        ],
        "responses": {
          "200": {
            "description": "Agent details",
            "content": {
              "application/json": {
                "schema": {
                  "$ref": "#/components/schemas/Agent"
                }
              }
            }
          }
        }
      },
      "delete": {
        "summary": "Delete an agent",
        "operationId": "deleteAgent",
        "parameters": [
          {
            "name": "id",
            "in": "path",
            "required": true,
            "schema": {
              "type": "string"
            }
          }
        ],
        "responses": {
          "200": {
            "description": "Agent deleted"
          }
        }
      }
    },
    "/api/agents/{id}/message": {
      "post": {
        "summary": "Send a message to an agent",
        "operationId": "sendMessage",
        "parameters": [
          {
            "name": "id",
            "in": "path",
            "required": true,
            "schema": {
              "type": "string"
            }
          }
        ],
        "requestBody": {
          "required": true,
          "content": {
            "application/json": {
              "schema": {
                "type": "object",
                "properties": {
                  "message": {
                    "type": "string"
                  }
                }
              }
            }
          }
        },
        "responses": {
          "200": {
            "description": "Agent response",
            "content": {
              "application/json": {
                "schema": {
                  "$ref": "#/components/schemas/MessageResponse"
                }
              }
            }
          }
        }
      }
    },
    "/api/agents/{id}/message/stream": {
      "post": {
        "summary": "Stream a message response",
        "operationId": "streamMessage",
        "parameters": [
          {
            "name": "id",
            "in": "path",
            "required": true,
            "schema": {
              "type": "string"
            }
          }
        ],
        "requestBody": {
          "required": true,
          "content": {
            "application/json": {
              "schema": {
                "type": "object",
                "properties": {
                  "message": {
                    "type": "string"
                  }
                }
              }
            }
          }
        },
        "responses": {
          "200": {
            "description": "Streaming response",
            "content": {
              "text/event-stream": {
                "schema": {
                  "type": "string"
                }
              }
            }
          }
        }
      }
    },
    "/api/sessions": {
      "get": {
        "summary": "List all sessions",
        "operationId": "listSessions",
        "responses": {
          "200": {
            "description": "List of sessions"
          }
        }
      }
    },
    "/api/workflows": {
      "get": {
        "summary": "List all workflows",
        "operationId": "listWorkflows",
        "responses": {
          "200": {
            "description": "List of workflows"
          }
        }
      },
      "post": {
        "summary": "Create a workflow",
        "operationId": "createWorkflow",
        "responses": {
          "200": {
            "description": "Created workflow"
          }
        }
      }
    },
    "/api/skills": {
      "get": {
        "summary": "List all skills",
        "operationId": "listSkills",
        "responses": {
          "200": {
            "description": "List of skills"
          }
        }
      }
    },
    "/api/channels": {
      "get": {
        "summary": "List all channels",
        "operationId": "listChannels",
        "responses": {
          "200": {
            "description": "List of channels"
          }
        }
      }
    },
    "/api/tools": {
      "get": {
        "summary": "List all tools",
        "operationId": "listTools",
        "responses": {
          "200": {
            "description": "List of tools"
          }
        }
      }
    },
    "/api/models": {
      "get": {
        "summary": "List all models",
        "operationId": "listModels",
        "responses": {
          "200": {
            "description": "List of models"
          }
        }
      }
    },
    "/api/providers": {
      "get": {
        "summary": "List all providers",
        "operationId": "listProviders",
        "responses": {
          "200": {
            "description": "List of providers"
          }
        }
      }
    },
    "/api/memory/agents/{agent_id}/kv": {
      "get": {
        "summary": "Get all memory entries for an agent",
        "operationId": "getMemoryAll",
        "parameters": [
          {
            "name": "agent_id",
            "in": "path",
            "required": true,
            "schema": {
              "type": "string"
            }
          }
        ],
        "responses": {
          "200": {
            "description": "Memory entries"
          }
        }
      }
    },
    "/api/triggers": {
      "get": {
        "summary": "List all triggers",
        "operationId": "listTriggers",
        "responses": {
          "200": {
            "description": "List of triggers"
          }
        }
      },
      "post": {
        "summary": "Create a trigger",
        "operationId": "createTrigger",
        "responses": {
          "200": {
            "description": "Created trigger"
          }
        }
      }
    },
    "/api/schedules": {
      "get": {
        "summary": "List all schedules",
        "operationId": "listSchedules",
        "responses": {
          "200": {
            "description": "List of schedules"
          }
        }
      },
      "post": {
        "summary": "Create a schedule",
        "operationId": "createSchedule",
        "responses": {
          "200": {
            "description": "Created schedule"
          }
        }
      }
    }
  },
  "components": {
    "schemas": {
      "Agent": {
        "type": "object",
        "properties": {
          "id": {
            "type": "string"
          },
          "name": {
            "type": "string"
          },
          "template": {
            "type": "string"
          },
          "status": {
            "type": "string"
          }
        }
      },
      "MessageResponse": {
        "type": "object",
        "properties": {
          "response": {
            "type": "string"
          },
          "input_tokens": {
            "type": "integer"
          },
          "output_tokens": {
            "type": "integer"
          },
          "iterations": {
            "type": "integer"
          }
        }
      }
    }
  }
}