#!/bin/bash
set -euo pipefail

# =============================================================================
# iam_setup_innovaped.sh — IAM dedicado de P&D para o Bedrock (RAGnaRock/Nidhogg)
# -----------------------------------------------------------------------------
# Segue o molde de Innova_DEVOPs/bedrock_iam_setup.sh (que provisiona o user de
# PRODUÇÃO do INNOVA), mas com propósito oposto: aqui é LABORATÓRIO — precisa
# alcançar QUALQUER modelo do catálogo, porque o trabalho é justamente comparar.
#
# Cria (idempotente):
#   1. IAM user   `InnovaPeD`  (+ tags de rateio)
#   2. IAM policy `InnovaPeDBedrockLab` — Bedrock e SÓ Bedrock, sem escrita
#   3. Attach da policy ao user
#   4. Access key (limite de 2 por user; não recria se já houver)
#
# Separação obtida:
#   - do usuário PESSOAL (`Alexandre`, admin da conta)
#   - do usuário de PRODUÇÃO do INNOVA (`innova-bedrock-prod`)
#
# Requisito: AWS CLI autenticada com quem tenha iam:CreateUser/CreatePolicy/
# CreateAccessKey (o `Alexandre` tem — verificado por simulate-principal-policy).
#
# Sem argumentos: mostra este help (convenção do repo).
# =============================================================================

[ $# -eq 0 ] && { sed -n '3,26p' "$0"; echo; echo "Uso: $0 --aplicar   (executa de verdade)"; exit 0; }
[ "$1" != "--aplicar" ] && { echo "argumento desconhecido: $1 (use --aplicar)"; exit 1; }

USER_NAME="InnovaPeD"
POLICY_NAME="InnovaPeDBedrockLab"
PROFILE_LOCAL="innovaped"     # perfil em ~/.aws/credentials — NÃO mexe no default

echo "==> Conta AWS:"
ACCOUNT_ID=$(aws sts get-caller-identity --query 'Account' --output text)
echo "    $ACCOUNT_ID  (executando como: $(aws sts get-caller-identity --query 'Arn' --output text))"
POLICY_ARN="arn:aws:iam::${ACCOUNT_ID}:policy/${POLICY_NAME}"

# -----------------------------------------------------------------------------
# 1. Policy — Bedrock inteiro para LER e INVOCAR; nada de criar/apagar/gerenciar
# -----------------------------------------------------------------------------
POLICY_DOC=$(cat <<'JSON'
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Sid": "InvocarQualquerModeloDoCatalogo",
      "Effect": "Allow",
      "Action": [
        "bedrock:InvokeModel",
        "bedrock:InvokeModelWithResponseStream",
        "bedrock:Converse",
        "bedrock:ConverseStream"
      ],
      "Resource": [
        "arn:aws:bedrock:*::foundation-model/*",
        "arn:aws:bedrock:*:*:inference-profile/*",
        "arn:aws:bedrock:*:*:application-inference-profile/*"
      ]
    },
    {
      "Sid": "LerCatalogo",
      "Effect": "Allow",
      "Action": [
        "bedrock:ListFoundationModels",
        "bedrock:GetFoundationModel",
        "bedrock:ListInferenceProfiles",
        "bedrock:GetInferenceProfile"
      ],
      "Resource": "*"
    }
  ]
}
JSON
)

if aws iam get-policy --policy-arn "$POLICY_ARN" >/dev/null 2>&1; then
  echo "==> Policy $POLICY_NAME já existe — mantida"
else
  echo "==> Criando policy $POLICY_NAME"
  aws iam create-policy --policy-name "$POLICY_NAME" \
    --description "P&D RAGnaRock/Nidhogg: invocar e listar Bedrock. Sem escrita, sem outros servicos." \
    --policy-document "$POLICY_DOC" --query 'Policy.Arn' --output text
fi

# -----------------------------------------------------------------------------
# 2. User + tags. As tags são o que permite rastrear e ratear depois.
# -----------------------------------------------------------------------------
if aws iam get-user --user-name "$USER_NAME" >/dev/null 2>&1; then
  echo "==> User $USER_NAME já existe — mantido"
else
  echo "==> Criando user $USER_NAME"
  aws iam create-user --user-name "$USER_NAME" \
    --tags Key=Projeto,Value=RAGnaRock Key=Ambiente,Value=PeD \
           Key=CentroDeCusto,Value=InnovaPeD Key=Dono,Value=Alexandre \
    --query 'User.Arn' --output text
fi

echo "==> Anexando policy ao user"
aws iam attach-user-policy --user-name "$USER_NAME" --policy-arn "$POLICY_ARN"

# -----------------------------------------------------------------------------
# 3. Access key — só cria se não houver (o limite da AWS é 2 por user)
# -----------------------------------------------------------------------------
N_KEYS=$(aws iam list-access-keys --user-name "$USER_NAME" \
           --query 'length(AccessKeyMetadata)' --output text)
if [ "$N_KEYS" -gt 0 ]; then
  echo "==> User já tem $N_KEYS access key(s). NÃO criei outra."
  echo "    (o secret só é exibido na criação; se perdeu, apague a antiga e rode de novo)"
  aws iam list-access-keys --user-name "$USER_NAME" \
    --query 'AccessKeyMetadata[].[AccessKeyId,Status,CreateDate]' --output table
  exit 0
fi

echo "==> Criando access key"
CRED=$(aws iam create-access-key --user-name "$USER_NAME" \
         --query 'AccessKey.[AccessKeyId,SecretAccessKey]' --output text)
AK=$(echo "$CRED" | cut -f1); SK=$(echo "$CRED" | cut -f2)

# -----------------------------------------------------------------------------
# 4. Grava como PERFIL SEPARADO — o default (pessoal) fica intocado
# -----------------------------------------------------------------------------
aws configure set aws_access_key_id     "$AK" --profile "$PROFILE_LOCAL"
aws configure set aws_secret_access_key "$SK" --profile "$PROFILE_LOCAL"
aws configure set region                "us-east-1" --profile "$PROFILE_LOCAL"
aws configure set output                "json"      --profile "$PROFILE_LOCAL"

echo
echo "==> PRONTO. Perfil local '$PROFILE_LOCAL' gravado em ~/.aws/credentials"
echo "    AWS_ACCESS_KEY_ID = $AK"
echo "    (o secret ficou só no arquivo — não é reexibível)"
echo
echo "    Testar:  aws sts get-caller-identity --profile $PROFILE_LOCAL"
echo "    Usar:    AWS_PROFILE=$PROFILE_LOCAL python3 tools/bedrock.py models us-east-1"
